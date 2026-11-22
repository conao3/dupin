use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, View, fuzzy};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAction {
    Sleep,
    LockScreen,
}

impl SystemAction {
    fn id(self) -> &'static str {
        match self {
            Self::Sleep => "system.sleep",
            Self::LockScreen => "system.lock-screen",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Sleep => "Sleep",
            Self::LockScreen => "Lock Screen",
        }
    }
}

pub struct SystemProvider;

impl Provider for SystemProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.query.trim().is_empty() || request.is_cancelled() {
            return Ok(());
        }
        let mut items: Vec<_> = [SystemAction::Sleep, SystemAction::LockScreen]
            .into_iter()
            .filter_map(|action| {
                let found = fuzzy::score(request.query.trim(), action.title())?;
                Some(Item {
                    id: action.id().into(),
                    view: View::Text {
                        title: action.title().into(),
                        subtitle: "System command · activate to run".into(),
                    },
                    record_usage: false,
                    icon: None,
                    matched: found.positions,
                    completion: None,
                    score: Some(found.score),
                    primary: Some("run".into()),
                    alternate: None,
                    actions: vec![ItemAction::new(
                        "run",
                        action.title(),
                        Action::Host(action.id().into()),
                    )],
                })
            })
            .collect();
        items.sort_by(|a, b| b.score.cmp(&a.score));
        items.truncate(request.limit);
        if !request.is_cancelled() {
            emit(items);
        }
        Ok(())
    }
}

pub fn dispatch(id: &str, runner: impl FnOnce(SystemAction) -> Result<(), String>) -> Result<(), String> {
    let action = match id {
        "system.sleep" => SystemAction::Sleep,
        "system.lock-screen" => SystemAction::LockScreen,
        _ => return Err("Unknown host action".into()),
    };
    runner(action).map_err(|error| format!("{} could not be carried out: {error}", action.title()))
}

pub fn run(action: SystemAction) -> Result<(), String> {
    match action {
        SystemAction::Sleep => {
            let status = std::process::Command::new("/usr/bin/pmset")
                .arg("sleepnow")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|error| error.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("pmset exited with {status}"))
            }
        }
        SystemAction::LockScreen => lock_screen(),
    }
}

fn lock_screen() -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightPostEventAccess() -> bool;
        fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *const c_void;
        fn CGEventSetFlags(event: *const c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *const c_void);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const c_void);
    }
    // Preflight never requests access. Posting the standard lock shortcut requires
    // existing event-synthesis access; private loginwindow APIs are not an alternative.
    unsafe {
        if !CGPreflightPostEventAccess() {
            return Err("Event-synthesis access is unavailable; no permission was requested".into());
        }
        let down = CGEventCreateKeyboardEvent(std::ptr::null(), 12, true);
        let up = CGEventCreateKeyboardEvent(std::ptr::null(), 12, false);
        if down.is_null() || up.is_null() {
            for event in [down, up] {
                if !event.is_null() {
                    CFRelease(event);
                }
            }
            return Err("Could not create the lock shortcut".into());
        }
        for event in [down, up] {
            CGEventSetFlags(event, (1 << 18) | (1 << 20));
            CGEventPost(0, event);
            CFRelease(event);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_is_inert_and_exposes_only_explicit_actions() {
        for (query, expected) in [
            ("", None),
            ("  ", None),
            ("restart", None),
            ("sleep", Some(SystemAction::Sleep)),
            ("lock", Some(SystemAction::LockScreen)),
        ] {
            let mut items = Vec::new();
            SystemProvider
                .search(&Request::detached(query, 2), &mut |batch| items.extend(batch))
                .unwrap();
            if let Some(action) = expected {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].action("run"), Some(&Action::Host(action.id().into())));
                assert!(items[0].completion.is_none());
                assert!(items[0].alternate.is_none());
                assert!(!items[0].record_usage);
            } else {
                assert!(items.is_empty());
            }
        }
        SystemProvider
            .search(&Request::detached("sleep", 0), &mut |_| panic!("zero limit"))
            .unwrap();
    }

    #[test]
    fn dispatch_is_typed_and_propagates_refusal_without_retry() {
        for action in [SystemAction::Sleep, SystemAction::LockScreen] {
            let mut calls = 0;
            let result = dispatch(action.id(), |received| {
                calls += 1;
                assert_eq!(received, action);
                Err("fixture refusal".into())
            });
            assert_eq!(calls, 1);
            assert!(result.unwrap_err().contains("fixture refusal"));
            assert!(
                dispatch(action.id(), |received| {
                    assert_eq!(received, action);
                    Ok(())
                })
                .is_ok()
            );
        }
        for id in ["", "sleep", "system.sleep;anything", "system.restart"] {
            assert!(dispatch(id, |_| panic!("unknown action reached runner")).is_err());
        }
    }
}
