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
use dupin_plugin_policy::{Access, Capability, PluginHandle, Policy};
use dupin_plugin_protocol::Operation;
#[cfg(feature = "wasm-runtime")]
use dupin_plugin_protocol::{Outcome, VERSION};
#[cfg(feature = "wasm-runtime")]
use dupin_plugin_runtime::{GuestModule, HostError, MAX_MODULE_BYTES, QueryControl};

pub struct CapabilityDescriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    kind: Capability,
}

pub struct Descriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub command: &'static str,
    pub default_enabled: bool,
    pub capabilities: &'static [CapabilityDescriptor],
    backend: Backend,
}

enum Backend {
    NativeSystem,
    #[cfg(feature = "wasm-runtime")]
    Guest {
        command: &'static str,
        module: &'static str,
    },
}

pub const DEADLINE: Duration = Duration::from_secs(2);

pub fn descriptors() -> &'static [Descriptor] {
    static PLUGINS: &[Descriptor] = &[
        #[cfg(feature = "wasm-runtime")]
        Descriptor {
            id: "calculator",
            title: "Calculator",
            description: "Arithmetic and unit conversions alongside app search",
            command: "calculator.evaluate",
            default_enabled: true,
            backend: Backend::Guest {
                command: "calculator",
                module: "calculator.wasm",
            },
            capabilities: &[CapabilityDescriptor {
                id: "clipboard-write",
                title: "Copy results",
                description: "Allow this plugin to write a result to the clipboard when you choose Copy",
                kind: Capability::ClipboardWrite,
            }],
        },
        Descriptor {
            id: "system",
            title: "System Commands",
            description: "Trusted in-process native reference plugin for sleep and screen locking",
            command: "system.commands",
            default_enabled: false,
            backend: Backend::NativeSystem,
            capabilities: &[
                CapabilityDescriptor {
                    id: "system-sleep",
                    title: "Sleep",
                    description: "Allow Sleep Now when explicitly selected",
                    kind: Capability::SystemSleep,
                },
                CapabilityDescriptor {
                    id: "system-lock-screen",
                    title: "Lock screen",
                    description: "Allow Lock Screen Now when explicitly selected",
                    kind: Capability::SystemLockScreen,
                },
            ],
        },
    ];
    PLUGINS
}

pub fn settings(config: &Config, plugin: &Descriptor) -> PluginSettings {
    config
        .plugins
        .get(plugin.id)
        .cloned()
        .unwrap_or_else(|| PluginSettings {
            enabled: plugin.default_enabled,
            grants: Vec::new(),
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
                let handle = policy.install(plugin.capabilities.iter().map(|cap| cap.kind));
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
            for capability in plugin.capabilities {
                policy.set_grant(
                    &entry.handle,
                    capability.kind,
                    settings.grants.iter().any(|id| id == capability.id),
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
                        Backend::NativeSystem => Arc::new(NativeProvider { enabled }) as Arc<dyn Provider>,
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
        let is_copy = matches!(operation, Operation::CopyText { .. });
        PreparedOperation::prepare(&self.policy.lock().unwrap(), &entry.handle, operation).map_err(|error| {
            match (is_copy, error) {
                (true, PrepareError::Access(Access::NeedsGrant)) => {
                    "Allow this plugin to copy results in Settings → Plugins."
                }
                (true, _) => "This action is no longer available.",
                _ => "Enable this plugin and allow this action in Settings → Plugins.",
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
    CommandId::new(plugin.command).expect("bundled command IDs are valid")
}

struct NativeProvider {
    enabled: Arc<AtomicBool>,
}
impl Provider for NativeProvider {
    fn search(
        &self,
        request: &Request,
        emit: &mut dyn FnMut(Vec<dupin_core::Item>),
    ) -> Result<(), ProviderError> {
        if !self.enabled.load(Ordering::Acquire) {
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
        if !self.enabled.load(Ordering::Acquire) || request.limit == 0 || request.is_cancelled() {
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
            command: match self.plugin.backend {
                Backend::Guest { command, .. } => command.into(),
                _ => return Err(ProviderError),
            },
            query: request.query.clone(),
            limit: request.limit.min(dupin_plugin_protocol::MAX_RESULTS as usize) as u32,
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
                        .map(|action| {
                            ItemAction::new(
                                &action.id,
                                &action.title,
                                Action::Plugin {
                                    command: command.clone(),
                                    operation: action.operation,
                                },
                            )
                        })
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
                .join(plugin.id)
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
    fn copying_requires_an_explicit_grant_and_enablement() {
        let mut config = Config::default();
        let plugin = &descriptors()[0];
        let command = command_id(plugin);
        let mut services = Services::new(&config);
        let operation = || Operation::CopyText { text: "42".into() };
        assert!(
            services
                .copy(&command, operation(), || true, |_| panic!("no grant"))
                .is_err()
        );
        config.plugins.insert(
            plugin.id.into(),
            PluginSettings {
                enabled: true,
                grants: vec!["clipboard-write".into()],
            },
        );
        services.apply_config(&config);
        let mut clipboard = String::new();
        services
            .copy(&command, operation(), || true, |text| clipboard = text.to_owned())
            .unwrap();
        assert_eq!(clipboard, "42");
        assert!(
            services
                .copy(&command, operation(), || false, |_| panic!("cancelled"))
                .is_err()
        );
        config.plugins.get_mut(plugin.id).unwrap().enabled = false;
        services.apply_config(&config);
        assert!(
            services
                .copy(&command, operation(), || true, |_| panic!("disabled"))
                .is_err()
        );
        config.plugins.get_mut(plugin.id).unwrap().enabled = true;
        config.plugins.get_mut(plugin.id).unwrap().grants.clear();
        services.apply_config(&config);
        assert!(
            services
                .copy(&command, operation(), || true, |_| panic!("revoked"))
                .is_err()
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
                grants: vec!["system-sleep".into()],
            },
        );
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
