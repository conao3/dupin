//! User-requested login registration, independent of application configuration.
use objc2::{
    MainThreadMarker, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_foundation::NSString;

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
    Error(String),
}

impl Status {
    fn from_raw(value: isize) -> Self {
        match value {
            0 => Self::NotRegistered,
            1 => Self::Enabled,
            2 => Self::RequiresApproval,
            3 => Self::NotFound,
            _ => Self::Error(format!("Unknown login item status: {value}")),
        }
    }

    pub fn action(&self) -> Option<Action> {
        match self {
            Self::NotRegistered => Some(Action::Register),
            Self::Enabled => Some(Action::Unregister),
            Self::RequiresApproval => Some(Action::OpenSettings),
            Self::NotFound | Self::Error(_) => None,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::NotRegistered => "Off",
            Self::Enabled => "On",
            Self::RequiresApproval => "Requires approval in System Settings",
            Self::NotFound => "Login item not found. Use an installed, signed Dupin app.",
            Self::Error(error) => error,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Register,
    Unregister,
    OpenSettings,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Register => "Enable",
            Self::Unregister => "Disable",
            Self::OpenSettings => "Open System Settings",
        }
    }
}

fn service() -> Result<Retained<AnyObject>, String> {
    let class = AnyClass::get(c"SMAppService").ok_or("Login items require macOS 13 or later")?;
    // SAFETY: SMAppService's class property returns an Objective-C object.
    let service: Option<Retained<AnyObject>> = unsafe { msg_send![class, mainAppService] };
    service.ok_or_else(|| "Login item unavailable".into())
}

/// Read the OS's current registration; never registers as a side effect.
pub fn status(_: MainThreadMarker) -> Status {
    match service() {
        Ok(service) => Status::from_raw(unsafe { msg_send![&*service, status] }),
        Err(error) => Status::Error(error),
    }
}

/// Perform only an explicit user action, then read back the actual OS status.
pub fn perform(action: Action, mtm: MainThreadMarker) -> (Status, Option<String>) {
    perform_with(
        action,
        |action| {
            let service = service()?;
            if action == Action::OpenSettings {
                let class = AnyClass::get(c"SMAppService").ok_or("Login items unavailable")?;
                unsafe {
                    let _: () = msg_send![class, openSystemSettingsLoginItems];
                }
                return Ok(());
            }
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let success: Bool = unsafe {
                match action {
                    Action::Register => msg_send![&*service, registerAndReturnError: &mut error],
                    Action::Unregister => msg_send![&*service, unregisterAndReturnError: &mut error],
                    Action::OpenSettings => unreachable!(),
                }
            };
            if success.as_bool() {
                return Ok(());
            }
            if error.is_null() {
                return Err("Could not change login registration".into());
            }
            let description: Retained<NSString> = unsafe { msg_send![error, localizedDescription] };
            Err(description.to_string())
        },
        || status(mtm),
    )
}

fn perform_with(
    action: Action,
    mutate: impl FnOnce(Action) -> Result<(), String>,
    read: impl FnOnce() -> Status,
) -> (Status, Option<String>) {
    let error = mutate(action).err();
    (read(), error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_states_select_only_valid_actions() {
        for (raw, expected, action) in [
            (0, Status::NotRegistered, Some(Action::Register)),
            (1, Status::Enabled, Some(Action::Unregister)),
            (2, Status::RequiresApproval, Some(Action::OpenSettings)),
            (3, Status::NotFound, None),
        ] {
            let status = Status::from_raw(raw);
            assert_eq!(status, expected);
            assert_eq!(status.action(), action);
        }
        assert!(matches!(Status::from_raw(99), Status::Error(_)));
        assert_eq!(Status::from_raw(99).action(), None);
    }
    #[test]
    fn successful_registration_does_not_imply_enabled() {
        let result = perform_with(
            Action::Register,
            |action| {
                assert_eq!(action, Action::Register);
                Ok(())
            },
            || Status::RequiresApproval,
        );
        assert_eq!(result, (Status::RequiresApproval, None));
    }
    #[test]
    fn explicit_settings_and_unregister_actions_read_back_without_assumptions() {
        for action in [Action::OpenSettings, Action::Unregister] {
            let mut calls = Vec::new();
            let result = perform_with(
                action,
                |selected| {
                    calls.push(selected);
                    Ok(())
                },
                || Status::NotRegistered,
            );
            assert_eq!(calls, [action]);
            assert_eq!(result, (Status::NotRegistered, None));
        }
    }

    #[test]
    fn failure_preserves_error_and_reads_actual_status() {
        let result = perform_with(
            Action::Unregister,
            |action| {
                assert_eq!(action, Action::Unregister);
                Err("denied".into())
            },
            || Status::Enabled,
        );
        assert_eq!(result, (Status::Enabled, Some("denied".into())));
    }
}
