use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use dupin_plugin_protocol::manifest::{Capability, Tier};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    Undecided,
    Allowed,
    Denied,
    Restricted,
}

#[derive(Clone, Debug)]
pub struct PluginHandle {
    authority: Arc<()>,
    instance: u64,
}

#[derive(Debug)]
pub struct Permit {
    plugin: PluginHandle,
    capability: Capability,
    revision: u64,
    consent_revision: u64,
}

#[derive(Debug)]
#[must_use]
pub enum Access {
    Allowed(Permit),
    MissingPlugin,
    Disabled,
    Undeclared,
    NeedsGrant,
    NeedsOsConsent,
    OsDenied,
    OsRestricted,
}

struct Grant {
    allowed: bool,
    revision: u64,
}

struct Plugin {
    enabled: bool,
    grants: BTreeMap<Capability, Grant>,
}

pub struct Policy {
    authority: Arc<()>,
    sequence: u64,
    plugins: BTreeMap<u64, Plugin>,
    calendar_consent: Consent,
    consent_revision: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            authority: Arc::new(()),
            sequence: 0,
            plugins: BTreeMap::new(),
            calendar_consent: Consent::Undecided,
            consent_revision: 0,
        }
    }
}

impl Policy {
    pub fn install_manifest(
        &mut self,
        manifest: &dupin_plugin_protocol::manifest::ValidatedManifest,
    ) -> PluginHandle {
        self.install(manifest.capabilities.iter().map(|cap| cap.kind.clone()))
    }

    fn revision(&mut self) -> u64 {
        self.sequence = self.sequence.checked_add(1).expect("policy revision exhausted");
        self.sequence
    }

    fn owns(&self, handle: &PluginHandle) -> bool {
        Arc::ptr_eq(&self.authority, &handle.authority)
    }

    pub fn install(&mut self, declarations: impl IntoIterator<Item = Capability>) -> PluginHandle {
        let instance = self.revision();
        let grants = declarations
            .into_iter()
            .map(|cap| {
                (
                    cap,
                    Grant {
                        allowed: false,
                        revision: instance,
                    },
                )
            })
            .collect();
        self.plugins.insert(
            instance,
            Plugin {
                enabled: false,
                grants,
            },
        );
        PluginHandle {
            authority: self.authority.clone(),
            instance,
        }
    }

    pub fn update_manifest(
        &mut self,
        handle: &PluginHandle,
        manifest: &dupin_plugin_protocol::manifest::ValidatedManifest,
    ) -> bool {
        self.update(handle, manifest.capabilities.iter().map(|cap| cap.kind.clone()))
    }

    pub fn uninstall(&mut self, handle: &PluginHandle) -> bool {
        self.owns(handle) && self.plugins.remove(&handle.instance).is_some()
    }

    pub fn set_enabled(&mut self, handle: &PluginHandle, enabled: bool) -> bool {
        if !self.owns(handle) || !self.plugins.contains_key(&handle.instance) {
            return false;
        }
        let revision = self.revision();
        let plugin = self.plugins.get_mut(&handle.instance).unwrap();
        if plugin.enabled != enabled {
            plugin.enabled = enabled;
            for grant in plugin.grants.values_mut() {
                grant.revision = revision;
            }
        }
        true
    }

    pub fn set_grant(&mut self, handle: &PluginHandle, cap: Capability, allowed: bool) -> bool {
        if !self.owns(handle) || !self.plugins.contains_key(&handle.instance) {
            return false;
        }
        let revision = self.revision();
        let Some(grant) = self
            .plugins
            .get_mut(&handle.instance)
            .unwrap()
            .grants
            .get_mut(&cap)
        else {
            return false;
        };
        if grant.allowed != allowed {
            grant.allowed = allowed;
            grant.revision = revision;
        }
        true
    }

    pub fn update(
        &mut self,
        handle: &PluginHandle,
        declarations: impl IntoIterator<Item = Capability>,
    ) -> bool {
        if !self.owns(handle) || !self.plugins.contains_key(&handle.instance) {
            return false;
        }
        let revision = self.revision();
        let declared: BTreeSet<_> = declarations.into_iter().collect();
        let plugin = self.plugins.get_mut(&handle.instance).unwrap();
        plugin.grants.retain(|cap, _| declared.contains(cap));
        for cap in declared {
            plugin.grants.entry(cap).or_insert(Grant {
                allowed: false,
                revision,
            });
        }
        true
    }

    pub fn set_calendar_consent(&mut self, consent: Consent) {
        if self.calendar_consent != consent {
            self.consent_revision = self.revision();
            self.calendar_consent = consent;
        }
    }

    pub fn access(&self, handle: &PluginHandle, cap: Capability) -> Access {
        if !self.owns(handle) {
            return Access::MissingPlugin;
        }
        let Some(plugin) = self.plugins.get(&handle.instance) else {
            return Access::MissingPlugin;
        };
        if !plugin.enabled {
            return Access::Disabled;
        }
        let Some(grant) = plugin.grants.get(&cap) else {
            return Access::Undeclared;
        };
        if !grant.allowed {
            return Access::NeedsGrant;
        }
        if cap == Capability::CalendarRead {
            match self.calendar_consent {
                Consent::Undecided => return Access::NeedsOsConsent,
                Consent::Denied => return Access::OsDenied,
                Consent::Restricted => return Access::OsRestricted,
                Consent::Allowed => {}
            }
        }
        Access::Allowed(Permit {
            plugin: handle.clone(),
            capability: cap.clone(),
            revision: grant.revision,
            consent_revision: if cap == Capability::CalendarRead {
                self.consent_revision
            } else {
                0
            },
        })
    }

    pub fn url_access(&self, handle: &PluginHandle, url: &str) -> (Option<Capability>, Access) {
        if dupin_plugin_protocol::url_scope::validated_url(url).is_err() {
            return (None, Access::Undeclared);
        }
        if !self.owns(handle) {
            return (None, Access::MissingPlugin);
        }
        let Some(plugin) = self.plugins.get(&handle.instance) else {
            return (None, Access::MissingPlugin);
        };
        if !plugin.enabled {
            return (None, Access::Disabled);
        }
        let mut denied = (None, Access::Undeclared);
        for cap in plugin.grants.keys() {
            if let Capability::OpenUrl(scope) = cap
                && scope.allows(url)
            {
                let access = self.access(handle, cap.clone());
                if matches!(access, Access::Allowed(_)) {
                    return (Some(cap.clone()), access);
                }
                denied = (Some(cap.clone()), access);
            }
        }
        denied
    }

    #[must_use]
    pub fn validate(&self, permit: &Permit, cap: Capability) -> bool {
        if permit.capability != cap {
            return false;
        }
        matches!(self.access(&permit.plugin, cap.clone()), Access::Allowed(current)
            if current.revision == permit.revision && current.consent_revision == permit.consent_revision)
    }
}
