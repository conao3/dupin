use std::collections::HashMap;
#[cfg(feature = "wasm-runtime")]
use std::io::Read;
#[cfg(feature = "wasm-runtime")]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(feature = "wasm-runtime")]
use std::{sync::OnceLock, time::Instant};

use dupin_core::config::PluginSettings;
#[cfg(feature = "wasm-runtime")]
use dupin_core::{Action, Item, ItemAction};
use dupin_core::{CommandId, Config, Provider};
use dupin_core::{ProviderError, Request};
use dupin_plugin_broker::{PrepareError, PreparedOperation};
use dupin_plugin_policy::{Access, Capability, PluginHandle, Policy, Tier};
use dupin_plugin_protocol::Operation;
#[cfg(feature = "wasm-runtime")]
use dupin_plugin_protocol::{Outcome, VERSION};
#[cfg(feature = "wasm-runtime")]
use dupin_plugin_runtime::{GuestModule, HostError, MAX_MODULE_BYTES, QueryControl};

use dupin_plugin_protocol::manifest::{CapabilityDescriptor, Manifest, ValidatedManifest, validate_catalog};
pub fn capability_allowed(capability: &CapabilityDescriptor, settings: &PluginSettings) -> bool {
    match capability.kind.tier() {
        Tier::Visible => !settings.revoked.iter().any(|id| id == &capability.id),
        Tier::Data | Tier::Privileged => settings.grants.iter().any(|id| id == &capability.id),
    }
}

pub fn capability_label(capability: Capability) -> &'static str {
    match capability {
        Capability::ClipboardWrite => "Copy to clipboard",
        Capability::CalendarRead => "Read calendar events",
        Capability::SystemSleep => "Put the Mac to sleep",
        Capability::SystemLockScreen => "Lock the screen",
    }
}

#[cfg(feature = "wasm-runtime")]
fn action_label(operation: &Operation) -> String {
    match operation {
        Operation::CopyText { text } => {
            let mut chars = text.chars();
            let mut preview: String = chars.by_ref().take(32).collect();
            if chars.next().is_some() {
                preview.push('…');
            }
            format!("Copy “{preview}”")
        }
        Operation::SystemCommand { command } => match command {
            dupin_plugin_protocol::SystemCommand::Sleep => "Sleep Now".into(),
            dupin_plugin_protocol::SystemCommand::LockScreen => "Lock Screen Now".into(),
        },
    }
}

#[cfg(feature = "wasm-runtime")]
fn guest_action(command: &CommandId, action: dupin_plugin_protocol::PluginAction) -> ItemAction {
    ItemAction::new(
        &action.id,
        &action_label(&action.operation),
        Action::Plugin {
            command: command.clone(),
            operation: action.operation,
        },
    )
}

pub struct Descriptor {
    manifest: ValidatedManifest,
    backend: Backend,
}
impl std::ops::Deref for Descriptor {
    type Target = ValidatedManifest;
    fn deref(&self) -> &ValidatedManifest {
        &self.manifest
    }
}

enum Backend {
    NativeSystem,
    #[cfg(feature = "wasm-runtime")]
    Guest {
        module: &'static str,
    },
}

pub const DEADLINE: Duration = Duration::from_secs(2);

pub fn descriptors() -> &'static [Descriptor] {
    static PLUGINS: std::sync::OnceLock<Vec<Descriptor>> = std::sync::OnceLock::new();
    PLUGINS.get_or_init(|| {
        let plugins = vec![
            #[cfg(feature = "wasm-runtime")]
            Descriptor {
                manifest: Manifest::decode(include_bytes!("calculator-manifest.json"))
                    .expect("valid calculator manifest"),
                backend: Backend::Guest {
                    module: "calculator.wasm",
                },
            },
            Descriptor {
                manifest: Manifest::decode(include_bytes!("system-manifest.json"))
                    .expect("valid system manifest"),
                backend: Backend::NativeSystem,
            },
        ];
        validate_catalog(
            &plugins.iter().map(|p| p.manifest.clone()).collect::<Vec<_>>(),
            &[],
        )
        .expect("unique bundled manifests");
        plugins
    })
}

pub fn settings(config: &Config, plugin: &Descriptor) -> PluginSettings {
    config
        .plugins
        .get(plugin.id.as_str())
        .cloned()
        .unwrap_or_else(|| PluginSettings {
            enabled: plugin.default_enabled,
            grants: Vec::new(),
            revoked: Vec::new(),
        })
}

struct Installed {
    handle: PluginHandle,
    enabled: Arc<AtomicBool>,
}

pub struct Services {
    policy: Arc<Mutex<Policy>>,
    installed: HashMap<CommandId, Installed>,
}

impl Services {
    pub fn new(config: &Config) -> Self {
        let mut policy = Policy::default();
        let installed = descriptors()
            .iter()
            .map(|plugin| {
                let handle = policy.install_manifest(&plugin.manifest);
                (
                    command_id(plugin),
                    Installed {
                        handle,
                        enabled: Arc::new(AtomicBool::new(false)),
                    },
                )
            })
            .collect();
        let mut services = Self {
            policy: Arc::new(Mutex::new(policy)),
            installed,
        };
        services.apply_config(config);
        services
    }

    pub fn apply_config(&mut self, config: &Config) {
        let mut policy = self.policy.lock().unwrap();
        for plugin in descriptors() {
            let entry = &self.installed[&command_id(plugin)];
            let settings = settings(config, plugin);
            policy.set_enabled(&entry.handle, settings.enabled);
            entry.enabled.store(settings.enabled, Ordering::Release);
            for capability in &plugin.capabilities {
                policy.set_grant(
                    &entry.handle,
                    capability.kind,
                    capability_allowed(capability, &settings),
                );
            }
        }
    }

    pub fn providers(&self) -> HashMap<CommandId, Arc<dyn Provider>> {
        descriptors()
            .iter()
            .map(|plugin| {
                let command = command_id(plugin);
                let enabled = self.installed[&command].enabled.clone();
                (
                    command,
                    match plugin.backend {
                        Backend::NativeSystem => {
                            Arc::new(NativeProvider { enabled, plugin }) as Arc<dyn Provider>
                        }
                        #[cfg(feature = "wasm-runtime")]
                        Backend::Guest { .. } => Arc::new(GuestProvider {
                            plugin,
                            enabled,
                            module: OnceLock::new(),
                        }) as Arc<dyn Provider>,
                    },
                )
            })
            .collect()
    }

    pub fn prepare(
        &self,
        command: &CommandId,
        operation: Operation,
    ) -> Result<PreparedOperation, &'static str> {
        let entry = self.installed.get(command).ok_or("This plugin is unavailable.")?;
        let plugin = descriptors()
            .iter()
            .find(|p| command_id(p) == *command)
            .ok_or("This plugin is unavailable.")?;
        if !plugin.allows_operation(&operation) {
            return Err("Undeclared plugin operation.");
        }
        let is_copy = matches!(operation, Operation::CopyText { .. });
        PreparedOperation::prepare(&self.policy.lock().unwrap(), &entry.handle, operation).map_err(|error| {
            match (is_copy, error) {
                (true, PrepareError::Access(Access::NeedsGrant)) => {
                    "Copy is turned off for this plugin in Settings → Plugins."
                }
                (true, _) => "This action is no longer available.",
                (false, PrepareError::Access(Access::Disabled | Access::NeedsGrant)) => {
                    "Enable this plugin and allow this action in Settings → Plugins."
                }
                _ => "This action is no longer available.",
            }
        })
    }

    pub fn policy(&self) -> Arc<Mutex<Policy>> {
        self.policy.clone()
    }

    #[cfg(all(test, feature = "wasm-runtime"))]
    pub fn copy(
        &self,
        command: &CommandId,
        operation: Operation,
        is_current: impl FnOnce() -> bool,
        sink: impl FnOnce(&str),
    ) -> Result<(), &'static str> {
        if !matches!(operation, Operation::CopyText { .. }) {
            return Err("Unsupported operation.");
        }
        let copy = self.prepare(command, operation)?;
        copy.dispatch(&self.policy.lock().unwrap(), is_current, |operation| {
            if let Operation::CopyText { text } = operation {
                sink(&text);
            }
        })
        .map_err(|_| "This action is no longer available.")
    }
}

pub fn command_id(plugin: &Descriptor) -> CommandId {
    CommandId::new(plugin.command.clone()).expect("bundled command IDs are valid")
}

struct NativeProvider {
    plugin: &'static Descriptor,
    enabled: Arc<AtomicBool>,
}
impl Provider for NativeProvider {
    fn search(
        &self,
        request: &Request,
        emit: &mut dyn FnMut(Vec<dupin_core::Item>),
    ) -> Result<(), ProviderError> {
        if !self.enabled.load(Ordering::Acquire) || !self.plugin.eligibility.accepts(&request.query) {
            return Ok(());
        }
        dupin_mac::system_commands::SystemProvider.search(request, emit)
    }
}

#[cfg(feature = "wasm-runtime")]
struct GuestProvider {
    plugin: &'static Descriptor,
    enabled: Arc<AtomicBool>,
    module: OnceLock<Result<GuestModule, HostError>>,
}

#[cfg(feature = "wasm-runtime")]
impl Provider for GuestProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if !self.enabled.load(Ordering::Acquire)
            || !self.plugin.eligibility.accepts(&request.query)
            || request.limit == 0
            || request.is_cancelled()
        {
            return Ok(());
        }
        let module = self
            .module
            .get_or_init(|| load_module(self.plugin))
            .as_ref()
            .map_err(|_| ProviderError)?;
        let message = dupin_plugin_protocol::Request {
            version: VERSION,
            request_id: request.generation().to_string(),
            generation: request.generation().to_string(),
            command: self.plugin.entrypoint.clone(),
            query: request.query.clone(),
            limit: request.limit.min(self.plugin.result_limit as usize) as u32,
            state: None,
        };
        let response = match module.query_controlled(
            &message,
            &QueryControl {
                deadline: Instant::now() + DEADLINE,
                is_cancelled: &|| request.is_cancelled() || !self.enabled.load(Ordering::Acquire),
            },
        ) {
            Ok(response) => response,
            Err(HostError::Fuel | HostError::Deadline | HostError::Cancelled) => return Ok(()),
            Err(_) => return Err(ProviderError),
        };
        let Outcome::Results { items, .. } = response.outcome else {
            return Err(ProviderError);
        };
        if !accepts_guest_items(&self.plugin.manifest, &items) {
            return Err(ProviderError);
        }
        let command = command_id(self.plugin);
        emit(
            items
                .into_iter()
                .map(|item| Item {
                    id: item.id,
                    view: item.view,
                    primary: item.primary,
                    alternate: None,
                    actions: item
                        .actions
                        .into_iter()
                        .map(|action| guest_action(&command, action))
                        .collect(),
                    completion: None,
                    record_usage: false,
                    icon: None,
                    matched: Vec::new(),
                    score: None,
                })
                .collect(),
        );
        Ok(())
    }
}

#[cfg(any(test, feature = "wasm-runtime"))]
fn accepts_guest_items(manifest: &ValidatedManifest, items: &[dupin_plugin_protocol::PluginItem]) -> bool {
    items.len() <= manifest.result_limit as usize
        && items.iter().all(|item| {
            manifest.allows_view(&item.view)
                && item
                    .actions
                    .iter()
                    .all(|action| manifest.allows_operation(&action.operation))
        })
}

#[cfg(feature = "wasm-runtime")]
fn load_module(plugin: &Descriptor) -> Result<GuestModule, HostError> {
    let path = module_path(plugin).ok_or(HostError::InvalidModule)?;
    let file = std::fs::File::open(path).map_err(|_| HostError::InvalidModule)?;
    let mut bytes = Vec::new();
    file.take(MAX_MODULE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| HostError::InvalidModule)?;
    GuestModule::load(&bytes)
}

#[cfg(feature = "wasm-runtime")]
fn module_path(plugin: &Descriptor) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins")
                .join(&plugin.id)
                .join("target/wasm32-unknown-unknown/release")
                .join(format!("dupin_{}.wasm", plugin.id)),
        );
    }
    bundled_module_path(
        &std::env::current_exe().ok()?,
        match plugin.backend {
            Backend::Guest { module, .. } => module,
            _ => return None,
        },
    )
}

#[cfg(feature = "wasm-runtime")]
fn bundled_module_path(executable: &Path, module: &str) -> Option<PathBuf> {
    let executable = std::fs::canonicalize(executable).ok()?;
    Some(
        executable
            .parent()?
            .parent()?
            .join("Resources/plugins")
            .join(module),
    )
}

#[cfg(all(test, feature = "wasm-runtime"))]
mod tests {
    use super::*;

    #[test]
    fn bundled_modules_resolve_from_the_app_and_the_nix_bin_symlink() {
        let root = std::env::temp_dir().join(format!(
            "dupin-plugin-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let contents = root.join("Applications/Dupin.app/Contents");
        let executable = contents.join("MacOS/dupin");
        let module = contents.join("Resources/plugins/calculator.wasm");
        let alias = root.join("bin/dupin");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::create_dir_all(alias.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"fixture").unwrap();
        std::fs::write(&module, b"fixture").unwrap();
        std::os::unix::fs::symlink("../Applications/Dupin.app/Contents/MacOS/dupin", &alias).unwrap();
        let expected = std::fs::canonicalize(&module).unwrap();
        let direct = bundled_module_path(&executable, "calculator.wasm");
        let through_alias = bundled_module_path(&alias, "calculator.wasm");
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(direct, Some(expected.clone()));
        assert_eq!(through_alias, Some(expected));
    }

    #[test]
    fn copy_is_allowed_when_enabled_and_stops_when_revoked_or_disabled() {
        let mut config = Config::default();
        let plugin = &descriptors()[0];
        let command = command_id(plugin);
        let mut services = Services::new(&config);
        let operation = || Operation::CopyText { text: "42".into() };
        let mut clipboard = String::new();
        services
            .copy(&command, operation(), || true, |text| clipboard = text.to_owned())
            .unwrap();
        assert_eq!(clipboard, "42");
        config.plugins.insert(
            plugin.id.clone(),
            PluginSettings {
                enabled: true,
                grants: vec!["clipboard-write".into()],
                revoked: vec!["clipboard-write".into()],
            },
        );
        services.apply_config(&config);
        assert_eq!(
            services.copy(&command, operation(), || true, |_| panic!("revoked")),
            Err("Copy is turned off for this plugin in Settings → Plugins.")
        );
        config
            .plugins
            .get_mut(plugin.id.as_str())
            .unwrap()
            .revoked
            .clear();
        services.apply_config(&config);
        let pending = PreparedOperation::prepare(
            &services.policy.lock().unwrap(),
            &services.installed[&command].handle,
            operation(),
        )
        .unwrap();
        config
            .plugins
            .get_mut(plugin.id.as_str())
            .unwrap()
            .revoked
            .push("clipboard-write".into());
        services.apply_config(&config);
        config
            .plugins
            .get_mut(plugin.id.as_str())
            .unwrap()
            .revoked
            .clear();
        services.apply_config(&config);
        assert!(
            pending
                .dispatch(
                    &services.policy.lock().unwrap(),
                    || true,
                    |_| panic!("stale permit")
                )
                .is_err()
        );
        assert!(
            services
                .copy(&command, operation(), || false, |_| panic!("cancelled"))
                .is_err()
        );
        config.plugins.get_mut(plugin.id.as_str()).unwrap().enabled = false;
        services.apply_config(&config);
        assert!(
            services
                .copy(&command, operation(), || true, |_| panic!("disabled"))
                .is_err()
        );
        config.plugins.get_mut(plugin.id.as_str()).unwrap().enabled = true;
        config
            .plugins
            .get_mut(plugin.id.as_str())
            .unwrap()
            .revoked
            .push("clipboard-write".into());
        services.apply_config(&config);
        assert!(
            services
                .copy(&command, operation(), || true, |_| panic!("revoked"))
                .is_err()
        );
    }

    #[test]
    fn data_access_still_requires_an_explicit_grant() {
        let capability = CapabilityDescriptor {
            id: "calendar-read".into(),
            kind: Capability::CalendarRead,
        };
        let mut settings = PluginSettings {
            enabled: true,
            grants: vec![],
            revoked: vec!["calendar-read".into()],
        };
        assert!(!capability_allowed(&capability, &settings));
        let mut policy = Policy::default();
        let handle = policy.install([Capability::CalendarRead]);
        policy.set_enabled(&handle, true);
        policy.set_grant(
            &handle,
            capability.kind,
            capability_allowed(&capability, &settings),
        );
        assert!(matches!(
            policy.access(&handle, capability.kind),
            Access::NeedsGrant
        ));
        settings.grants.push("calendar-read".into());
        assert!(capability_allowed(&capability, &settings));
        assert_eq!(settings.revoked, ["calendar-read"]);
    }

    #[test]
    fn guest_action_title_cannot_rename_the_host_action() {
        let action = guest_action(
            &command_id(&descriptors()[0]),
            dupin_plugin_protocol::PluginAction {
                id: "copy".into(),
                title: "Open Safari".into(),
                operation: Operation::CopyText { text: "3".into() },
            },
        );
        assert_eq!(action.title, "Copy “3”");
        assert_eq!(
            action_label(&Operation::CopyText {
                text: "日".repeat(33)
            }),
            format!("Copy “{}…”", "日".repeat(32))
        );
    }

    #[test]
    fn disabled_and_zero_limit_queries_do_not_load_a_module() {
        let provider = GuestProvider {
            plugin: &descriptors()[0],
            enabled: Arc::new(AtomicBool::new(false)),
            module: OnceLock::new(),
        };
        provider
            .search(&Request::detached("2+2", 1), &mut |_| panic!("disabled result"))
            .unwrap();
        assert!(provider.module.get().is_none());
        provider.enabled.store(true, Ordering::Release);
        provider
            .search(&Request::detached("2^1000000000", 0), &mut |_| {
                panic!("zero limit result")
            })
            .unwrap();
        assert!(provider.module.get().is_none());
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    fn system_plugin_starts_disabled_and_each_grant_is_separate() {
        let mut config = Config::default();
        let mut services = Services::new(&config);
        let command = CommandId::new("system.commands").unwrap();
        let sleep = || Operation::SystemCommand {
            command: dupin_plugin_protocol::SystemCommand::Sleep,
        };
        let lock = || Operation::SystemCommand {
            command: dupin_plugin_protocol::SystemCommand::LockScreen,
        };
        assert!(services.prepare(&command, sleep()).is_err());
        services.providers()[&command]
            .search(&Request::detached("sleep", 2), &mut |_| panic!("disabled"))
            .unwrap();
        config.plugins.insert(
            "system".into(),
            PluginSettings {
                enabled: true,
                grants: vec![],
                revoked: vec!["system-sleep".into()],
            },
        );
        services.apply_config(&config);
        assert!(services.prepare(&command, sleep()).is_err());
        assert!(services.prepare(&command, lock()).is_err());
        config
            .plugins
            .get_mut("system")
            .unwrap()
            .grants
            .push("system-sleep".into());
        services.apply_config(&config);
        assert!(services.prepare(&command, sleep()).is_ok());
        assert!(services.prepare(&command, lock()).is_err());
        let mut rows = Vec::new();
        services.providers()[&command]
            .search(&Request::detached("sleep", 2), &mut |batch| rows.extend(batch))
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].primary.is_none());
        let prepared = services.prepare(&command, sleep()).unwrap();
        config.plugins.get_mut("system").unwrap().grants.clear();
        services.apply_config(&config);
        config
            .plugins
            .get_mut("system")
            .unwrap()
            .grants
            .push("system-sleep".into());
        services.apply_config(&config);
        assert!(
            prepared
                .dispatch(&services.policy.lock().unwrap(), || true, |_| panic!("stale"))
                .is_err()
        );
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::*;
    #[test]
    fn registration_defaults_and_limits_match_both_builds() {
        let plugins = descriptors();
        assert_eq!(plugins.len(), if cfg!(feature = "wasm-runtime") { 2 } else { 1 });
        let system = plugins.iter().find(|p| p.id == "system").unwrap();
        assert!(!settings(&Config::default(), system).enabled);
        assert_eq!(system.result_limit, 2);
        assert!(settings(&Config::default(), system).grants.is_empty());
        #[cfg(feature = "wasm-runtime")]
        {
            let calc = plugins.iter().find(|p| p.id == "calculator").unwrap();
            assert!(settings(&Config::default(), calc).enabled);
            assert_eq!(calc.result_limit, 1);
            assert!(settings(&Config::default(), calc).grants.is_empty());
        }
    }
    #[test]
    fn guest_results_are_checked_before_actions_are_exposed() {
        use dupin_plugin_protocol::{PluginAction, PluginItem, SystemCommand, View};
        let manifest = Manifest::decode(include_bytes!("calculator-manifest.json")).unwrap();
        let mut item = PluginItem {
            id: "answer".into(),
            view: View::Pair {
                source: "1+1".into(),
                result: "2".into(),
            },
            primary: None,
            actions: vec![],
        };
        assert!(accepts_guest_items(&manifest, &[item.clone()]));
        item.view = View::Text {
            title: "unexpected".into(),
            subtitle: String::new(),
        };
        assert!(!accepts_guest_items(&manifest, &[item.clone()]));
        item.view = View::Pair {
            source: "1+1".into(),
            result: "2".into(),
        };
        item.actions.push(PluginAction {
            id: "run".into(),
            title: "Sleep".into(),
            operation: Operation::SystemCommand {
                command: SystemCommand::Sleep,
            },
        });
        assert!(!accepts_guest_items(&manifest, &[item]));
    }
    #[test]
    fn host_rejects_operations_outside_manifest_even_before_grants() {
        let services = Services::new(&Config::default());
        let system = descriptors().iter().find(|p| p.id == "system").unwrap();
        assert!(matches!(
            services.prepare(
                &command_id(system),
                Operation::CopyText {
                    text: "result".into()
                }
            ),
            Err("Undeclared plugin operation.")
        ));
    }
}
