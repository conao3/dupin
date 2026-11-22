use std::collections::HashMap;
#[cfg(feature = "wasm-runtime")]
use std::io::Read;
#[cfg(feature = "wasm-runtime")]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
#[cfg(feature = "wasm-runtime")]
use std::{sync::OnceLock, time::Instant};

use dupin_core::config::PluginSettings;
#[cfg(feature = "wasm-runtime")]
use dupin_core::{Action, Item, ItemAction, ProviderError, Request};
use dupin_core::{CommandId, Config, Provider};
use dupin_plugin_broker::{PrepareError, PreparedCopy};
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
    #[cfg(feature = "wasm-runtime")]
    guest_command: &'static str,
    #[cfg(feature = "wasm-runtime")]
    module: &'static str,
}

pub const DEADLINE: Duration = Duration::from_secs(2);

#[cfg(feature = "wasm-runtime")]
pub fn descriptors() -> &'static [Descriptor] {
    static PLUGINS: &[Descriptor] = &[Descriptor {
        id: "calculator",
        title: "Calculator",
        description: "Arithmetic and unit conversions alongside app search",
        command: "calculator.evaluate",
        default_enabled: true,
        guest_command: "calculator",
        module: "calculator.wasm",
        capabilities: &[CapabilityDescriptor {
            id: "clipboard-write",
            title: "Copy results",
            description: "Allow this plugin to write a result to the clipboard when you choose Copy",
            kind: Capability::ClipboardWrite,
        }],
    }];
    PLUGINS
}

#[cfg(not(feature = "wasm-runtime"))]
pub fn descriptors() -> &'static [Descriptor] {
    &[]
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
    policy: Policy,
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
        let mut services = Self { policy, installed };
        services.apply_config(config);
        services
    }

    pub fn apply_config(&mut self, config: &Config) {
        for plugin in descriptors() {
            let entry = &self.installed[&command_id(plugin)];
            let settings = settings(config, plugin);
            self.policy.set_enabled(&entry.handle, settings.enabled);
            entry.enabled.store(settings.enabled, Ordering::Release);
            for capability in plugin.capabilities {
                self.policy.set_grant(
                    &entry.handle,
                    capability.kind,
                    settings.grants.iter().any(|id| id == capability.id),
                );
            }
        }
    }

    pub fn providers(&self) -> HashMap<CommandId, Arc<dyn Provider>> {
        #[cfg(not(feature = "wasm-runtime"))]
        return HashMap::new();

        #[cfg(feature = "wasm-runtime")]
        descriptors()
            .iter()
            .map(|plugin| {
                let command = command_id(plugin);
                let enabled = self.installed[&command].enabled.clone();
                (
                    command,
                    Arc::new(GuestProvider {
                        plugin,
                        enabled,
                        module: OnceLock::new(),
                    }) as Arc<dyn Provider>,
                )
            })
            .collect()
    }

    pub fn copy(
        &self,
        command: &CommandId,
        operation: Operation,
        is_current: impl FnOnce() -> bool,
        sink: impl FnOnce(&str),
    ) -> Result<(), &'static str> {
        let entry = self.installed.get(command).ok_or("This plugin is unavailable.")?;
        let copy =
            PreparedCopy::prepare(&self.policy, &entry.handle, operation).map_err(|error| match error {
                PrepareError::Access(Access::NeedsGrant) => {
                    "Allow this plugin to copy results in Settings → Plugins."
                }
                _ => "This action is no longer available.",
            })?;
        copy.dispatch(&self.policy, is_current, sink)
            .map_err(|_| "This action is no longer available.")
    }
}

pub fn command_id(plugin: &Descriptor) -> CommandId {
    CommandId::new(plugin.command).expect("bundled command IDs are valid")
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
            command: self.plugin.guest_command.into(),
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
    Some(
        std::env::current_exe()
            .ok()?
            .parent()?
            .parent()?
            .join("Resources/plugins")
            .join(plugin.module),
    )
}

#[cfg(all(test, feature = "wasm-runtime"))]
mod tests {
    use super::*;

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
