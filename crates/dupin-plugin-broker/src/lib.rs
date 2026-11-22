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
pub struct PreparedCopy {
    permit: Permit,
    text: String,
}

impl PreparedCopy {
    pub fn prepare(
        policy: &Policy,
        plugin: &PluginHandle,
        operation: Operation,
    ) -> Result<Self, PrepareError> {
        let Operation::CopyText { text } = operation;
        if text.is_empty() || text.len() > MAX_COPY_BYTES {
            return Err(PrepareError::InvalidOperation);
        }
        match policy.access(plugin, Capability::ClipboardWrite) {
            Access::Allowed(permit) => Ok(Self { permit, text }),
            access => Err(PrepareError::Access(access)),
        }
    }

    pub fn dispatch(
        self,
        policy: &Policy,
        is_current: impl FnOnce() -> bool,
        copy: impl FnOnce(&str),
    ) -> Result<(), DispatchError> {
        if !is_current() {
            return Err(DispatchError::Cancelled);
        }
        if !policy.validate(&self.permit, Capability::ClipboardWrite) {
            return Err(DispatchError::AccessChanged);
        }
        copy(&self.text);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn invocation_requires_its_own_plugin_grant() {
        let mut policy = Policy::default();
        let first = granted(&mut policy);
        let second = enabled(&mut policy);
        assert!(matches!(
            PreparedCopy::prepare(&policy, &second, operation()),
            Err(PrepareError::Access(Access::NeedsGrant))
        ));
        let copy = PreparedCopy::prepare(&policy, &first, operation()).unwrap();
        let mut clipboard = String::new();
        copy.dispatch(&policy, || true, |text| clipboard = text.to_owned())
            .unwrap();
        assert_eq!(clipboard, "approx. 37.7777777778 °C");
    }

    #[test]
    fn revoked_or_disabled_work_cannot_reach_the_sink_after_regrant() {
        for disable in [false, true] {
            let mut policy = Policy::default();
            let plugin = granted(&mut policy);
            let copy = PreparedCopy::prepare(&policy, &plugin, operation()).unwrap();
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
        let copy = PreparedCopy::prepare(&policy, &plugin, operation()).unwrap();
        assert_eq!(
            copy.dispatch(&policy, || false, |_| panic!("cancelled copy")),
            Err(DispatchError::Cancelled)
        );
        let copy = PreparedCopy::prepare(&policy, &plugin, operation()).unwrap();
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
                PreparedCopy::prepare(&policy, &plugin, Operation::CopyText { text }),
                Err(PrepareError::InvalidOperation)
            ));
        }
        assert!(
            PreparedCopy::prepare(
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
