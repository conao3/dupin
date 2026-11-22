use super::*;
use Capability::{CalendarRead, ClipboardWrite};

fn permit(policy: &Policy, plugin: &PluginHandle, cap: Capability) -> Permit {
    let Access::Allowed(permit) = policy.access(plugin, cap) else {
        panic!("expected permit");
    };
    permit
}

fn enabled(policy: &mut Policy, declarations: &[Capability]) -> PluginHandle {
    let plugin = policy.install(declarations.iter().copied());
    assert!(policy.set_enabled(&plugin, true));
    plugin
}

#[test]
fn declarations_enablement_grants_and_os_consent_are_distinct() {
    let mut policy = Policy::default();
    let plugin = policy.install([CalendarRead]);
    assert!(matches!(policy.access(&plugin, CalendarRead), Access::Disabled));
    policy.set_enabled(&plugin, true);
    assert!(matches!(policy.access(&plugin, CalendarRead), Access::NeedsGrant));
    policy.set_grant(&plugin, CalendarRead, true);
    assert!(matches!(
        policy.access(&plugin, CalendarRead),
        Access::NeedsOsConsent
    ));
    policy.set_calendar_consent(Consent::Allowed);
    assert!(policy.validate(&permit(&policy, &plugin, CalendarRead), CalendarRead));
    assert!(matches!(
        policy.access(&plugin, ClipboardWrite),
        Access::Undeclared
    ));
    assert!(!policy.set_grant(&plugin, ClipboardWrite, true));
}

#[test]
fn shared_os_consent_never_grants_another_plugin_access() {
    let mut policy = Policy::default();
    let first = enabled(&mut policy, &[CalendarRead]);
    let second = enabled(&mut policy, &[CalendarRead]);
    policy.set_grant(&first, CalendarRead, true);
    policy.set_calendar_consent(Consent::Allowed);
    assert!(policy.validate(&permit(&policy, &first, CalendarRead), CalendarRead));
    assert!(matches!(policy.access(&second, CalendarRead), Access::NeedsGrant));
}

#[test]
fn denied_and_restricted_consent_do_not_request_another_prompt() {
    let mut policy = Policy::default();
    let plugin = enabled(&mut policy, &[CalendarRead]);
    policy.set_grant(&plugin, CalendarRead, true);
    policy.set_calendar_consent(Consent::Denied);
    assert!(matches!(policy.access(&plugin, CalendarRead), Access::OsDenied));
    policy.set_calendar_consent(Consent::Restricted);
    assert!(matches!(
        policy.access(&plugin, CalendarRead),
        Access::OsRestricted
    ));
}

#[test]
fn clipboard_write_needs_its_own_grant_but_no_calendar_consent() {
    let mut policy = Policy::default();
    let plugin = enabled(&mut policy, &[ClipboardWrite]);
    assert!(matches!(
        policy.access(&plugin, ClipboardWrite),
        Access::NeedsGrant
    ));
    policy.set_grant(&plugin, ClipboardWrite, true);
    let token = permit(&policy, &plugin, ClipboardWrite);
    policy.set_calendar_consent(Consent::Denied);
    assert!(policy.validate(&token, ClipboardWrite));
    assert!(!policy.validate(&token, CalendarRead));
}

#[test]
fn updates_preserve_unchanged_grants_and_block_added_capabilities() {
    let mut policy = Policy::default();
    let plugin = enabled(&mut policy, &[ClipboardWrite]);
    policy.set_grant(&plugin, ClipboardWrite, true);
    let token = permit(&policy, &plugin, ClipboardWrite);
    policy.update(&plugin, [ClipboardWrite, CalendarRead]);
    assert!(policy.validate(&token, ClipboardWrite));
    assert!(matches!(policy.access(&plugin, CalendarRead), Access::NeedsGrant));
    policy.update(&plugin, [CalendarRead]);
    assert!(!policy.validate(&token, ClipboardWrite));
    policy.update(&plugin, [CalendarRead, ClipboardWrite]);
    assert!(matches!(
        policy.access(&plugin, ClipboardWrite),
        Access::NeedsGrant
    ));
    policy.set_grant(&plugin, ClipboardWrite, true);
    assert!(!policy.validate(&token, ClipboardWrite));
}

#[test]
fn revocation_invalidates_only_the_affected_capability_and_cannot_be_undone() {
    let mut policy = Policy::default();
    let plugin = enabled(&mut policy, &[ClipboardWrite, CalendarRead]);
    policy.set_grant(&plugin, ClipboardWrite, true);
    policy.set_grant(&plugin, CalendarRead, true);
    policy.set_calendar_consent(Consent::Allowed);
    let copy = permit(&policy, &plugin, ClipboardWrite);
    let calendar = permit(&policy, &plugin, CalendarRead);
    policy.set_enabled(&plugin, true);
    policy.set_grant(&plugin, CalendarRead, true);
    assert!(policy.validate(&copy, ClipboardWrite));
    assert!(policy.validate(&calendar, CalendarRead));
    policy.set_grant(&plugin, CalendarRead, false);
    assert!(!policy.validate(&calendar, CalendarRead));
    assert!(policy.validate(&copy, ClipboardWrite));
    policy.set_grant(&plugin, CalendarRead, true);
    assert!(!policy.validate(&calendar, CalendarRead));
    assert!(policy.validate(&permit(&policy, &plugin, CalendarRead), CalendarRead));
    policy.update(&plugin, [ClipboardWrite]);
    assert!(policy.validate(&copy, ClipboardWrite));
}

#[test]
fn disable_and_reenable_do_not_revive_pending_operations() {
    let mut policy = Policy::default();
    let plugin = enabled(&mut policy, &[ClipboardWrite]);
    policy.set_grant(&plugin, ClipboardWrite, true);
    let token = permit(&policy, &plugin, ClipboardWrite);
    policy.set_enabled(&plugin, false);
    assert!(!policy.validate(&token, ClipboardWrite));
    policy.set_enabled(&plugin, true);
    assert!(!policy.validate(&token, ClipboardWrite));
    assert!(policy.validate(&permit(&policy, &plugin, ClipboardWrite), ClipboardWrite));
}

#[test]
fn os_revocation_and_regrant_do_not_revive_pending_operations() {
    let mut policy = Policy::default();
    let plugin = enabled(&mut policy, &[CalendarRead]);
    policy.set_grant(&plugin, CalendarRead, true);
    policy.set_calendar_consent(Consent::Allowed);
    let token = permit(&policy, &plugin, CalendarRead);
    policy.set_calendar_consent(Consent::Denied);
    assert!(!policy.validate(&token, CalendarRead));
    policy.set_calendar_consent(Consent::Allowed);
    assert!(!policy.validate(&token, CalendarRead));
}

#[test]
fn uninstall_and_foreign_policies_cannot_reuse_handles_or_permits() {
    let mut policy = Policy::default();
    let old = enabled(&mut policy, &[ClipboardWrite]);
    policy.set_grant(&old, ClipboardWrite, true);
    let token = permit(&policy, &old, ClipboardWrite);
    let mut foreign = Policy::default();
    let other = enabled(&mut foreign, &[ClipboardWrite]);
    foreign.set_grant(&other, ClipboardWrite, true);
    assert!(!foreign.validate(&token, ClipboardWrite));
    assert!(!foreign.set_grant(&old, ClipboardWrite, true));
    assert!(!foreign.set_enabled(&old, true));
    assert!(!foreign.update(&old, [ClipboardWrite]));
    assert!(!foreign.uninstall(&old));
    assert!(policy.uninstall(&old));
    let new = enabled(&mut policy, &[ClipboardWrite]);
    policy.set_grant(&new, ClipboardWrite, true);
    assert!(!policy.validate(&token, ClipboardWrite));
    assert!(matches!(
        policy.access(&old, ClipboardWrite),
        Access::MissingPlugin
    ));
}
