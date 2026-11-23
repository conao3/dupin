use std::collections::HashMap;
#[cfg(feature = "wasm-runtime")]
use std::io::Read;
#[cfg(feature = "wasm-runtime")]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
#[cfg(feature = "wasm-runtime")]
use std::{sync::OnceLock, time::Instant};

use dupin_core::config::PluginSettings;
use dupin_core::{Action, Item, ItemAction, ProviderError, Request};
use dupin_core::{CommandId, Config, History, Provider, QueryEligibility};
use dupin_plugin_broker::{PrepareError, PreparedCopy};
use dupin_plugin_policy::{Access, Capability, PluginHandle, Policy, Tier};
use dupin_plugin_protocol::Operation;
#[cfg(feature = "wasm-runtime")]
use dupin_plugin_protocol::{Outcome, VERSION};
#[cfg(feature = "wasm-runtime")]
use dupin_plugin_runtime::{GuestModule, HostError, MAX_MODULE_BYTES, QueryControl};

pub struct CapabilityDescriptor {
    pub id: &'static str,
    pub kind: Capability,
}

impl CapabilityDescriptor {
    pub fn allowed(&self, settings: &PluginSettings) -> bool {
        match self.kind.tier() {
            Tier::Visible => !settings.revoked.iter().any(|id| id == self.id),
            Tier::Data | Tier::Privileged => settings.grants.iter().any(|id| id == self.id),
        }
    }
}

pub fn capability_label(capability: Capability) -> &'static str {
    match capability {
        Capability::ClipboardWrite => "Copy to clipboard",
        Capability::CalendarRead => "Read calendar events",
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

enum Backend {
    Recent,
    #[cfg(feature = "wasm-runtime")]
    Guest(Guest),
}

#[cfg(feature = "wasm-runtime")]
struct Guest {
    command: &'static str,
    module: &'static str,
}

pub struct Descriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub command: &'static str,
    pub default_enabled: bool,
    pub capabilities: &'static [CapabilityDescriptor],
    pub query_eligibility: QueryEligibility,
    pub limit: usize,
    backend: Backend,
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
            query_eligibility: QueryEligibility::NonEmpty,
            limit: 1,
            backend: Backend::Guest(Guest {
                command: "calculator",
                module: "calculator.wasm",
            }),
            capabilities: &[CapabilityDescriptor {
                id: "clipboard-write",
                kind: Capability::ClipboardWrite,
            }],
        },
        Descriptor {
            id: "recent",
            title: "Recent",
            description: "Recent and frequent apps and files when the query is empty",
            command: "recent.items",
            default_enabled: true,
            query_eligibility: QueryEligibility::EmptyOnly,
            limit: 10,
            backend: Backend::Recent,
            capabilities: &[CapabilityDescriptor {
                id: "clipboard-write",
                kind: Capability::ClipboardWrite,
            }],
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
            revoked: Vec::new(),
        })
}

struct Installed {
    handle: PluginHandle,
    enabled: Arc<AtomicBool>,
}

pub struct Services {
    policy: Policy,
    installed: HashMap<CommandId, Installed>,
    history: Arc<RwLock<History>>,
}

impl Services {
    pub fn new(config: &Config, history: &History) -> Self {
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
            policy,
            installed,
            history: Arc::new(RwLock::new(history.clone())),
        };
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
                self.policy
                    .set_grant(&entry.handle, capability.kind, capability.allowed(&settings));
            }
        }
    }

    pub fn update_history(&self, history: &History) {
        *self.history.write().unwrap_or_else(|e| e.into_inner()) = history.clone();
    }

    pub fn providers(&self, apps: &Arc<dupin_mac::AppsProvider>) -> HashMap<CommandId, Arc<dyn Provider>> {
        descriptors()
            .iter()
            .map(|plugin| {
                let command = command_id(plugin);
                let provider: Arc<dyn Provider> = match &plugin.backend {
                    Backend::Recent => Arc::new(CopyBroker {
                        command: command.clone(),
                        provider: dupin_mac::RecentProvider::new(apps.clone(), self.history.clone()),
                    }),
                    #[cfg(feature = "wasm-runtime")]
                    Backend::Guest(guest) => Arc::new(GuestProvider {
                        plugin,
                        guest,
                        enabled: self.installed[&command].enabled.clone(),
                        module: OnceLock::new(),
                    }),
                };
                (command, provider)
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
                    "Copy is turned off for this plugin in Settings → Plugins."
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

struct CopyBroker<P> {
    command: CommandId,
    provider: P,
}

impl<P: Provider> Provider for CopyBroker<P> {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.provider.search(request, &mut |items| {
            let items = items
                .into_iter()
                .map(|mut item| {
                    let actions = item
                        .actions
                        .iter()
                        .filter_map(|action| {
                            let effect = match item.action(&action.id)? {
                                Action::Copy(text) => Action::Plugin {
                                    command: self.command.clone(),
                                    operation: Operation::CopyText { text: text.clone() },
                                },
                                _ => return Some(action.clone()),
                            };
                            Some(ItemAction::new(&action.id, &action.title, effect))
                        })
                        .collect();
                    item.actions = actions;
                    item
                })
                .collect();
            emit(items);
        })
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;

    struct Paths;

    impl Provider for Paths {
        fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
            let mut item = Item::path("/fixture/report.txt".into(), "Report".into(), Vec::new());
            item.actions.push(
                ItemAction::new(
                    "preview",
                    "Preview",
                    Action::Preview("/fixture/report.txt".into()),
                )
                .keep_open(),
            );
            emit(vec![item]);
            Ok(())
        }
    }

    #[test]
    fn native_copy_actions_follow_visible_opt_out_and_keep_other_action_semantics() {
        let plugin = descriptors().iter().find(|p| p.id == "recent").unwrap();
        let command = command_id(plugin);
        let provider = CopyBroker {
            command: command.clone(),
            provider: Paths,
        };
        let mut items = Vec::new();
        provider
            .search(&Request::detached("", 10), &mut |batch| items.extend(batch))
            .unwrap();
        assert!(matches!(items[0].action("open"), Some(Action::Open(_))));
        assert_eq!(
            items[0].item_action("preview"),
            Some(
                &ItemAction::new(
                    "preview",
                    "Preview",
                    Action::Preview("/fixture/report.txt".into())
                )
                .keep_open()
            )
        );
        let Some(Action::Plugin {
            command: origin,
            operation,
        }) = items[0].action("copy-path")
        else {
            panic!("native copy must use the plugin broker");
        };
        assert_eq!(origin, &command);
        let mut config = Config::default();
        let mut services = Services::new(&config, &History::default());
        services.copy(origin, operation.clone(), || true, |_| {}).unwrap();
        config.plugins.insert(
            plugin.id.into(),
            PluginSettings {
                enabled: true,
                grants: vec!["clipboard-write".into()],
                revoked: vec!["clipboard-write".into()],
            },
        );
        services.apply_config(&config);
        assert!(
            services
                .copy(origin, operation.clone(), || true, |_| panic!("revoked"))
                .is_err()
        );
        config.plugins.get_mut(plugin.id).unwrap().revoked.clear();
        services.apply_config(&config);
        let mut copied = String::new();
        services
            .copy(origin, operation.clone(), || true, |text| copied = text.into())
            .unwrap();
        assert_eq!(copied, "/fixture/report.txt");
        config.plugins.get_mut(plugin.id).unwrap().enabled = false;
        services.apply_config(&config);
        assert!(
            services
                .copy(origin, operation.clone(), || true, |_| panic!("disabled copy"))
                .is_err()
        );
    }
}

#[cfg(feature = "wasm-runtime")]
struct GuestProvider {
    plugin: &'static Descriptor,
    guest: &'static Guest,
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
            .get_or_init(|| load_module(self.plugin, self.guest))
            .as_ref()
            .map_err(|_| ProviderError)?;
        let message = dupin_plugin_protocol::Request {
            version: VERSION,
            request_id: request.generation().to_string(),
            generation: request.generation().to_string(),
            command: self.guest.command.into(),
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

#[cfg(feature = "wasm-runtime")]
fn load_module(plugin: &Descriptor, guest: &Guest) -> Result<GuestModule, HostError> {
    let path = module_path(plugin, guest).ok_or(HostError::InvalidModule)?;
    let file = std::fs::File::open(path).map_err(|_| HostError::InvalidModule)?;
    let mut bytes = Vec::new();
    file.take(MAX_MODULE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| HostError::InvalidModule)?;
    GuestModule::load(&bytes)
}

#[cfg(feature = "wasm-runtime")]
fn module_path(plugin: &Descriptor, guest: &Guest) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins")
                .join(plugin.id)
                .join("target/wasm32-unknown-unknown/release")
                .join(format!("dupin_{}.wasm", plugin.id)),
        );
    }
    bundled_module_path(&std::env::current_exe().ok()?, guest.module)
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
        let mut services = Services::new(&config, &History::default());
        let operation = || Operation::CopyText { text: "42".into() };
        let mut clipboard = String::new();
        services
            .copy(&command, operation(), || true, |text| clipboard = text.to_owned())
            .unwrap();
        assert_eq!(clipboard, "42");
        config.plugins.insert(
            plugin.id.into(),
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
        config.plugins.get_mut(plugin.id).unwrap().revoked.clear();
        services.apply_config(&config);
        let pending = PreparedCopy::prepare(
            &services.policy,
            &services.installed[&command].handle,
            operation(),
        )
        .unwrap();
        config
            .plugins
            .get_mut(plugin.id)
            .unwrap()
            .revoked
            .push("clipboard-write".into());
        services.apply_config(&config);
        config.plugins.get_mut(plugin.id).unwrap().revoked.clear();
        services.apply_config(&config);
        assert!(
            pending
                .dispatch(&services.policy, || true, |_| panic!("stale permit"))
                .is_err()
        );
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
        config
            .plugins
            .get_mut(plugin.id)
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
            id: "calendar-read",
            kind: Capability::CalendarRead,
        };
        let mut settings = PluginSettings {
            enabled: true,
            grants: vec![],
            revoked: vec!["calendar-read".into()],
        };
        assert!(!capability.allowed(&settings));
        let mut policy = Policy::default();
        let handle = policy.install([Capability::CalendarRead]);
        policy.set_enabled(&handle, true);
        policy.set_grant(&handle, capability.kind, capability.allowed(&settings));
        assert!(matches!(
            policy.access(&handle, capability.kind),
            Access::NeedsGrant
        ));
        settings.grants.push("calendar-read".into());
        assert!(capability.allowed(&settings));
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
        let plugin = &descriptors()[0];
        let Backend::Guest(guest) = &plugin.backend else {
            panic!("expected calculator guest")
        };
        let provider = GuestProvider {
            plugin,
            guest,
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
