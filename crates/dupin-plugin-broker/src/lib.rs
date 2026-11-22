use dupin_plugin_policy::{Access, Capability, Permit, PluginHandle, Policy};
use dupin_plugin_protocol::{MAX_COPY_BYTES, Operation};

#[derive(Debug)]
pub enum PrepareError {
    InvalidOperation,
    Access(Access),
}

#[derive(Debug, PartialEq, Eq)]
pub enum DispatchError {
    Cancelled,
    AccessChanged,
}

#[must_use]
pub struct PreparedOperation {
    permit: Permit,
    operation: Operation,
    capability: Capability,
}

impl PreparedOperation {
    pub fn prepare(
        policy: &Policy,
        plugin: &PluginHandle,
        operation: Operation,
    ) -> Result<Self, PrepareError> {
        if let Operation::CopyText { text } = &operation {
            if text.is_empty() || text.len() > MAX_COPY_BYTES {
                return Err(PrepareError::InvalidOperation);
            }
        }
        let capability = dupin_plugin_protocol::manifest::ActionKind::of(&operation).capability();
        match policy.access(plugin, capability) {
            Access::Allowed(permit) => Ok(Self {
                permit,
                operation,
                capability,
            }),
            access => Err(PrepareError::Access(access)),
        }
    }

    pub fn dispatch(
        self,
        policy: &Policy,
        is_current: impl FnOnce() -> bool,
        sink: impl FnOnce(Operation),
    ) -> Result<(), DispatchError> {
        if !is_current() {
            return Err(DispatchError::Cancelled);
        }
        if !policy.validate(&self.permit, self.capability) {
            return Err(DispatchError::AccessChanged);
        }
        sink(self.operation);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_plugin_protocol::SystemCommand;

    fn operation() -> Operation {
        Operation::CopyText {
            text: "approx. 37.7777777778 °C".into(),
        }
    }

    fn enabled(policy: &mut Policy) -> PluginHandle {
        let plugin = policy.install([Capability::ClipboardWrite]);
        policy.set_enabled(&plugin, true);
        plugin
    }

    fn granted(policy: &mut Policy) -> PluginHandle {
        let plugin = enabled(policy);
        policy.set_grant(&plugin, Capability::ClipboardWrite, true);
        plugin
    }

    #[test]
    fn system_operations_require_their_own_declaration_enablement_and_grant() {
        for (command, capability, other) in [
            (
                SystemCommand::Sleep,
                Capability::SystemSleep,
                Capability::SystemLockScreen,
            ),
            (
                SystemCommand::LockScreen,
                Capability::SystemLockScreen,
                Capability::SystemSleep,
            ),
        ] {
            let operation = || Operation::SystemCommand { command };
            let mut policy = Policy::default();
            let plugin = policy.install([capability]);
            assert!(matches!(
                PreparedOperation::prepare(&policy, &plugin, operation()),
                Err(PrepareError::Access(Access::Disabled))
            ));
            policy.set_enabled(&plugin, true);
            assert!(matches!(
                PreparedOperation::prepare(&policy, &plugin, operation()),
                Err(PrepareError::Access(Access::NeedsGrant))
            ));
            policy.set_grant(&plugin, capability, true);
            assert!(matches!(
                PreparedOperation::prepare(
                    &policy,
                    &plugin,
                    Operation::SystemCommand {
                        command: if command == SystemCommand::Sleep {
                            SystemCommand::LockScreen
                        } else {
                            SystemCommand::Sleep
                        }
                    }
                ),
                Err(PrepareError::Access(Access::Undeclared))
            ));
            let foreign = policy.install([capability, other]);
            policy.set_enabled(&foreign, true);
            assert!(PreparedOperation::prepare(&policy, &foreign, operation()).is_err());
            for disable in [false, true] {
                let prepared = PreparedOperation::prepare(&policy, &plugin, operation()).unwrap();
                if disable {
                    policy.set_enabled(&plugin, false);
                    policy.set_enabled(&plugin, true);
                } else {
                    policy.set_grant(&plugin, capability, false);
                    policy.set_grant(&plugin, capability, true);
                }
                assert_eq!(
                    prepared.dispatch(&policy, || true, |_| panic!("stale permit")),
                    Err(DispatchError::AccessChanged)
                );
            }
            let prepared = PreparedOperation::prepare(&policy, &plugin, operation()).unwrap();
            assert_eq!(
                prepared.dispatch(&policy, || false, |_| panic!("cancelled")),
                Err(DispatchError::Cancelled)
            );
            let prepared = PreparedOperation::prepare(&policy, &plugin, operation()).unwrap();
            assert_eq!(
                prepared.dispatch(&Policy::default(), || true, |_| panic!("replaced policy")),
                Err(DispatchError::AccessChanged)
            );
            let mut calls = 0;
            PreparedOperation::prepare(&policy, &plugin, operation())
                .unwrap()
                .dispatch(
                    &policy,
                    || true,
                    |received| {
                        assert_eq!(received, operation());
                        calls += 1;
                    },
                )
                .unwrap();
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn invocation_requires_its_own_plugin_grant() {
        let mut policy = Policy::default();
        let first = granted(&mut policy);
        let second = enabled(&mut policy);
        assert!(matches!(
            PreparedOperation::prepare(&policy, &second, operation()),
            Err(PrepareError::Access(Access::NeedsGrant))
        ));
        let copy = PreparedOperation::prepare(&policy, &first, operation()).unwrap();
        let mut clipboard = String::new();
        copy.dispatch(
            &policy,
            || true,
            |operation| {
                let Operation::CopyText { text } = operation else {
                    panic!()
                };
                clipboard = text;
            },
        )
        .unwrap();
        assert_eq!(clipboard, "approx. 37.7777777778 °C");
    }

    #[test]
    fn revoked_or_disabled_work_cannot_reach_the_sink_after_regrant() {
        for disable in [false, true] {
            let mut policy = Policy::default();
            let plugin = granted(&mut policy);
            let copy = PreparedOperation::prepare(&policy, &plugin, operation()).unwrap();
            if disable {
                policy.set_enabled(&plugin, false);
                policy.set_enabled(&plugin, true);
            } else {
                policy.set_grant(&plugin, Capability::ClipboardWrite, false);
                policy.set_grant(&plugin, Capability::ClipboardWrite, true);
            }
            assert_eq!(
                copy.dispatch(&policy, || true, |_| panic!("revoked copy")),
                Err(DispatchError::AccessChanged)
            );
        }
    }

    #[test]
    fn query_cancellation_and_policy_replacement_reject_pending_copies() {
        let mut policy = Policy::default();
        let plugin = granted(&mut policy);
        let copy = PreparedOperation::prepare(&policy, &plugin, operation()).unwrap();
        assert_eq!(
            copy.dispatch(&policy, || false, |_| panic!("cancelled copy")),
            Err(DispatchError::Cancelled)
        );
        let copy = PreparedOperation::prepare(&policy, &plugin, operation()).unwrap();
        assert_eq!(
            copy.dispatch(&Policy::default(), || true, |_| panic!("foreign copy")),
            Err(DispatchError::AccessChanged)
        );
    }

    #[test]
    fn direct_host_construction_cannot_bypass_payload_limits() {
        let mut policy = Policy::default();
        let plugin = granted(&mut policy);
        for text in [String::new(), "x".repeat(MAX_COPY_BYTES + 1)] {
            assert!(matches!(
                PreparedOperation::prepare(&policy, &plugin, Operation::CopyText { text }),
                Err(PrepareError::InvalidOperation)
            ));
        }
        assert!(
            PreparedOperation::prepare(
                &policy,
                &plugin,
                Operation::CopyText {
                    text: "x".repeat(MAX_COPY_BYTES)
                }
            )
            .is_ok()
        );
    }
}
