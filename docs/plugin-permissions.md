# Plugin permission policy

`dupin-plugin-policy` is a host-owned, in-memory policy component for #12 and #96.
The bundled Wasm calculator uses it through `dupin-plugin-broker` for its Copy
result action, and Settings persists and edits that plugin's enablement and
clipboard grant. The policy crate itself performs no OS operations. App, file and
dictionary providers remain trusted native code that accesses macOS directly;
they are not yet migrated to this broker. This is a permission boundary for the
wired action, not a sandbox for native code or a complete policy system for all
plugins.

Each installation receives an opaque handle belonging to one policy instance.
The host's installation registry owns its association with a public plugin ID;
the guest cannot supply a handle. New installations start disabled with declared
capabilities but no grants. Enablement and per-capability grants are separate
explicit operations. Neither requests OS access.

The policy defines clipboard write and calendar read. The calculator currently
uses clipboard write; the policy also tracks calendar consent, but no provider or
OS-consent flow is connected. Clipboard read is not available through a write
grant. File, network, process and URL capabilities are deliberately absent until
their resource scopes are defined.

The broker must call `access` for an actual operation. Calendar read can return
`NeedsOsConsent` only after enablement, declaration and a plugin grant. Denied and
restricted OS states return distinct decisions rather than another prompt request.
The host supplies OS consent status; global consent never grants another plugin
access. Clipboard write requires a plugin grant but has no calendar consent gate.

An allowed operation receives an opaque permit. Before releasing an asynchronous
completion or performing its effect, the broker must call `validate` with the
permit and the required capability, after refreshing relevant OS consent. The
host must prevent policy changes between validation and dispatch. The policy
cannot undo an effect already performed by the OS.

Revocation invalidates outstanding permits for that capability. Disabling
invalidates every permit for the plugin. Re-enabling or granting access again does
not revive old permits. Removed declarations lose their grants; adding them again
requires a new grant. Updates retain grants and permits for unchanged capabilities
while additions start ungranted. OS consent changes invalidate calendar permits.
Uninstalling invalidates the handle permanently, including after reinstalling the
same public plugin ID. Another policy instance cannot accept its handles or permits.

`dupin-plugin-broker` connects typed copy operations to this policy. It checks
payload bounds and clipboard-write access at preparation, then consumes the
prepared copy once after rechecking its permit and a host-supplied current-query
predicate. The launcher supplies the platform clipboard sink only after these
checks. Both the predicate and sink belong to the trusted host, never the guest.

The calculator's enabled state and grants are stored in `config.toml`; Settings
edits them without prompting macOS. Other plugins' migration and persistence,
calendar prompt coordination, additional broker operations and a full lifecycle
for installed plugins remain future work. Tests use synthetic policy state and
never request or change OS permissions.
