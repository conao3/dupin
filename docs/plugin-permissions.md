# Plugin permission policy

`dupin-plugin-policy` is a host-owned, in-memory policy component for #12 and #96.
The bundled Wasm calculator uses it through `dupin-plugin-broker` for its Copy
result action, and Settings persists and edits that plugin's enablement and
visible-action opt-outs. The policy crate itself performs no OS operations. App, file and
dictionary providers remain trusted native code that accesses macOS directly;
they are not yet migrated to this broker. This is a permission boundary for the
wired action, not a sandbox for native code or a complete policy system for all
plugins.

Each installation receives an opaque handle belonging to one policy instance.
The host's installation registry owns its association with a public plugin ID;
the guest cannot supply a handle. New installations start disabled with declared
capabilities. The host computes access from fixed capability tiers: Visible
actions are allowed while enabled unless revoked; Data and Privileged capabilities
need explicit grants. Neither enabling a plugin nor changing these settings requests OS access.

The policy defines clipboard write, system sleep, screen lock and calendar read. The calculator currently
uses clipboard write; the policy also tracks calendar consent, but no provider or
OS-consent flow is connected. Clipboard read is not available through a write
grant. File, network, process and URL capabilities are deliberately absent until
their resource scopes are defined.

The broker must call `access` for an actual operation. Calendar read can return
`NeedsOsConsent` only after enablement, declaration and a plugin grant. Denied and
restricted OS states return distinct decisions rather than another prompt request.
The host supplies OS consent status; global consent never grants another plugin
access. Clipboard write is Visible and has no calendar consent gate. It can be
turned off individually even while calculation remains enabled.

An allowed operation receives an opaque permit. Before releasing an asynchronous
completion or performing its effect, the broker must call `validate` with the
permit and the required capability, after refreshing relevant OS consent. The
host must prevent policy changes between validation and dispatch. The policy
cannot undo an effect already performed by the OS.

Revocation invalidates outstanding permits for that capability. Disabling
invalidates every permit for the plugin. Re-enabling or granting access again does
not revive old permits. Removed declarations lose their grants; adding them again
requires a new permit. Updates retain access for unchanged capabilities. New Data
and Privileged capabilities start ungranted; new bundled Visible actions are
allowed unless revoked. A future third-party installer must re-disclose new
Visible actions. OS consent changes invalidate calendar permits.
Uninstalling invalidates the handle permanently, including after reinstalling the
same public plugin ID. Another policy instance cannot accept its handles or permits.

`dupin-plugin-broker` connects typed operations to this policy. It checks
payload bounds and the operation-specific capability at preparation, then consumes
the prepared operation once after rechecking its permit and a host-supplied current-query
predicate. The launcher supplies the platform clipboard sink only after these
checks. Both the predicate and sink belong to the trusted host, never the guest.

The calculator's enabled state and opt-outs are stored in `config.toml`.
`grants` records explicit Data/Privileged allows; `revoked` records Visible
opt-outs. An id in the wrong list is ignored and preserved, including legacy
`grants = ["clipboard-write"]`. The earlier development format recorded only
positive grants, so an absent clipboard grant cannot distinguish never granting
from turning it off. That unreleased format has no refusal history to migrate.
Settings edits only the chosen id without prompting macOS. Other plugins' migration and persistence,
calendar prompt coordination, additional broker operations and a full lifecycle
for installed plugins remain future work. Tests use synthetic policy state and
never request or change OS permissions.

System Commands is a trusted in-process native reference plugin, not a sandbox.
It starts disabled and declares separate ungranted sleep and lock-screen
capabilities. Enabling it, granting either capability and focusing Settings do
not preflight OS access or request permissions. Search only produces actions;
Enter opens review before an explicit action selection.

The broker prepares a capability-specific revocable permit and revalidates it
with query cancellation immediately before worker handoff. Revocation, disabling
and regrant cannot revive a pending permit. The policy lock is released before
the fixed native operation runs off the UI thread. Once handed off, an accepted
OS operation cannot be undone; a late completion cannot dismiss a newer query.
Lock uses the existing fixed CoreGraphics shortcut and reports missing event
access without prompting.

Manual review remains required for full sleep/wake, PreventSystemSleep,
non-QWERTY layouts, remapped shortcuts, actual locking and OS permission refusal.
App, file and dictionary migration remains outstanding.
