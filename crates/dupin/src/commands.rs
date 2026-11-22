use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use dupin_core::{CommandId, CommandRegistry, CommandSpec, Config, Provider, QueryPresentation};
use dupin_mac::{AppsProvider, DictionaryProvider, SpotlightProvider, Target};

pub const APPS: &str = "apps.search";
pub const FILES: &str = "files.names";
pub const CONTENTS: &str = "files.contents";
pub const DICTIONARY: &str = "dictionary.lookup";
const WEB_KEYWORDS: &str = "web.keywords";
const WEB_FALLBACKS: &str = "web.fallbacks";
const MAX_FILE_CANDIDATES: usize = 10_000;

pub struct Registration {
    pub registry: CommandRegistry,
    pub providers: HashMap<CommandId, Arc<dyn Provider>>,
    pub notice: Option<String>,
    pub web: Arc<RwLock<crate::web::WebCatalog>>,
    pub file_demotions: Arc<RwLock<Vec<String>>>,
}

pub struct ConfiguredCommands {
    pub registry: CommandRegistry,
    pub notice: Option<String>,
    pub web: crate::web::WebCatalog,
}

fn parsed_file_query(
    command: &CommandId,
    raw: &str,
) -> Option<Result<dupin_core::file_query::FileQuery, dupin_core::file_query::FileQueryError>> {
    matches!(command.as_str(), FILES | CONTENTS).then(|| dupin_core::file_query::FileQuery::parse(raw))
}

pub fn query_qualifier_ranges(command: &CommandId, raw: &str) -> Vec<std::ops::Range<usize>> {
    parsed_file_query(command, raw)
        .and_then(Result::ok)
        .map(|query| query.qualifier_ranges)
        .unwrap_or_default()
}

pub fn query_problem(command: &CommandId, raw: &str) -> Option<&'static str> {
    match parsed_file_query(command, raw)? {
        Err(error) => Some(error.message()),
        Ok(query) if command.as_str() == CONTENTS && query.terms.is_empty() && !raw.trim().is_empty() => {
            Some("Type a word to search file contents.")
        }
        _ => None,
    }
}

fn id(value: &str) -> CommandId {
    CommandId::new(value).expect("built-in command IDs are valid")
}

fn presentation(
    placeholder: &str,
    chip: Option<&str>,
    empty_hint: Option<&str>,
    no_results_hint: Option<&str>,
    show_searching: bool,
    backdrop: f32,
) -> QueryPresentation {
    QueryPresentation {
        placeholder: placeholder.into(),
        chip: chip.map(str::to_owned),
        empty_hint: empty_hint.map(str::to_owned),
        no_results_hint: no_results_hint.map(str::to_owned),
        show_searching,
        backdrop,
    }
}

pub fn configured(config: &Config) -> Result<ConfiguredCommands, dupin_core::CommandError> {
    let file_limit = config.file_limit.min(MAX_FILE_CANDIDATES);
    let apps_id = id(APPS);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let dictionary_id = id(DICTIONARY);
    let mut registry = CommandRegistry::new(apps_id.clone(), Some(files_id.clone()));
    for command in [
        CommandSpec::new(
            apps_id.clone(),
            50,
            presentation("Search apps", None, None, Some("No matches"), false, 0.0),
        ),
        CommandSpec::new(
            files_id.clone(),
            file_limit,
            presentation(
                "Search files",
                Some("Files"),
                Some("Type to search files with Spotlight  ·  ⌫ back to apps"),
                Some("No matches"),
                true,
                1.0,
            ),
        )
        .with_candidate_limit(file_limit.saturating_mul(4).min(MAX_FILE_CANDIDATES)),
        CommandSpec::new(
            contents_id.clone(),
            file_limit,
            presentation(
                "Search inside files",
                Some("In files"),
                Some("Type to search inside files with Spotlight  ·  ⌫ back to apps"),
                Some("No matches"),
                true,
                2.0,
            ),
        ),
        CommandSpec::new(
            dictionary_id.clone(),
            1,
            presentation(
                "Look up a word",
                Some("Define"),
                Some("Type a word  ·  ⌫ back to apps"),
                Some("No definition found"),
                true,
                3.0,
            ),
        ),
    ] {
        registry.register(command)?;
    }

    registry.register(CommandSpec::new(
        id(WEB_KEYWORDS),
        1,
        presentation("Web search", None, None, None, false, 0.0),
    ))?;
    registry.register(
        CommandSpec::new(
            id(WEB_FALLBACKS),
            20,
            presentation("Web search", None, None, None, false, 0.0),
        )
        .with_fallback(),
    )?;

    registry.add_keyword(&contents_id, "in")?;
    let mut rejected = Vec::new();
    for keyword in &config.dictionary_keywords {
        if let Err(error) = registry.add_keyword(&dictionary_id, keyword) {
            rejected.push(error.to_string());
        }
    }
    let dictionary_notice = (!rejected.is_empty()).then(|| {
        format!(
            "Some dictionary keywords were not registered. Review dictionary_keywords in config.toml: {}",
            rejected.join("; "),
        )
    });

    let mut defaults = Vec::new();
    for plugin in crate::plugins::descriptors() {
        let command = crate::plugins::command_id(plugin);
        let mut spec = CommandSpec::new(
            command.clone(),
            1,
            presentation(plugin.title, Some(plugin.title), None, None, false, 0.0),
        );
        spec.deadline = crate::plugins::DEADLINE;
        registry.register(spec)?;
        if crate::plugins::settings(config, plugin).enabled {
            defaults.push(command);
        }
    }
    defaults.extend([id(APPS), id(WEB_KEYWORDS), id(WEB_FALLBACKS)]);
    registry.set_default_providers(defaults, Some(id(WEB_KEYWORDS)))?;
    let (web, web_notice) =
        crate::web::WebCatalog::compile(config, |keyword| registry.keyword(keyword).is_some());
    let notices: Vec<_> = [dictionary_notice, web_notice].into_iter().flatten().collect();
    let notice = (!notices.is_empty()).then(|| notices.join("\n"));
    Ok(ConfiguredCommands {
        registry,
        notice,
        web,
    })
}

pub fn register(
    config: &Config,
    apps: Arc<AppsProvider>,
    plugins: &crate::plugins::Services,
) -> Result<Registration, dupin_core::CommandError> {
    let ConfiguredCommands {
        registry,
        notice,
        web,
    } = configured(config)?;
    let web = Arc::new(RwLock::new(web));
    let apps_id = id(APPS);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let dictionary_id = id(DICTIONARY);
    let file_demotions = Arc::new(RwLock::new(config.file_demotions.clone()));
    let mut providers: HashMap<_, Arc<dyn Provider>> = [
        (apps_id, apps as Arc<dyn Provider>),
        (
            files_id,
            Arc::new(SpotlightProvider::with_demotions(
                Target::Names,
                file_demotions.clone(),
            )) as Arc<dyn Provider>,
        ),
        (
            contents_id,
            Arc::new(SpotlightProvider::new(Target::Contents)) as Arc<dyn Provider>,
        ),
        (
            dictionary_id,
            Arc::new(DictionaryProvider::default()) as Arc<dyn Provider>,
        ),
        (
            id(WEB_KEYWORDS),
            Arc::new(crate::web::WebProvider::new(web.clone(), false)) as Arc<dyn Provider>,
        ),
        (
            id(WEB_FALLBACKS),
            Arc::new(crate::web::WebProvider::new(web.clone(), true)) as Arc<dyn Provider>,
        ),
    ]
    .into();
    providers.extend(plugins.providers());

    Ok(Registration {
        registry,
        providers,
        notice,
        web,
        file_demotions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "wasm-runtime")]
    #[test]
    fn calculator_answers_precede_apps_and_explicit_web_keywords_precede_both() {
        use dupin_core::{Delivery, Effect, History, Item, Request, Session};

        let mut config = Config::default();
        config.web_searches.insert(
            "pi".into(),
            dupin_core::config::WebSearch {
                name: "Pi documentation".into(),
                url: "https://example.com/search?q={query}".into(),
            },
        );
        let plugin = crate::plugins::command_id(&crate::plugins::descriptors()[0]);
        for query in ["2+2", "pi"] {
            for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2]] {
                let configured = configured(&config).unwrap();
                let web = crate::web::WebProvider::new(Arc::new(RwLock::new(configured.web)), false);
                let mut session = Session::new(History::default(), configured.registry).unwrap();
                let effects = session.set_query(query);
                let generation = effects
                    .iter()
                    .find_map(|effect| match effect {
                        Effect::Search { generation, .. } => Some(*generation),
                        _ => None,
                    })
                    .unwrap();
                let answer = Item::path("/fixture/answer".into(), "Answer".into(), vec![]);
                let app = Item::path("/fixture/app".into(), "App".into(), vec![]);
                let mut web_items = Vec::new();
                web.search(&Request::detached(query, 1), &mut |rows| web_items.extend(rows))
                    .unwrap();
                assert_eq!(web_items.len(), usize::from(query == "pi"));
                let mut expected = vec![answer.id.clone(), app.id.clone()];
                if let Some(web_item) = web_items.first() {
                    expected.insert(0, web_item.id.clone());
                }
                let providers = [
                    (id(APPS), vec![app]),
                    (plugin.clone(), vec![answer]),
                    (id(WEB_KEYWORDS), web_items),
                ];
                for index in order {
                    let (origin, rows) = &providers[index];
                    let delivery = if rows.is_empty() {
                        Delivery::Done { generation }
                    } else {
                        Delivery::Batch {
                            generation,
                            items: rows.clone(),
                        }
                    };
                    assert!(session.deliver_from(origin.clone(), delivery));
                }
                assert_eq!(
                    session.items().iter().map(|item| &item.id).collect::<Vec<_>>(),
                    expected.iter().collect::<Vec<_>>()
                );
                assert_eq!(session.selected(), 0);
            }
        }
    }

    #[test]
    fn web_results_use_the_registered_session_action_and_completion_paths() {
        use dupin_core::{Action, Delivery, Effect, History, Intent, Request, Session};

        let configured = configured(&Config::default()).unwrap();
        let web = crate::web::WebProvider::new(Arc::new(RwLock::new(configured.web)), false);
        let mut session = Session::new(History::default(), configured.registry).unwrap();
        let effects = session.set_query("g");
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search { generation, .. } => Some(*generation),
                _ => None,
            })
            .unwrap();
        let mut rows = Vec::new();
        web.search(&Request::detached("g", 1), &mut |batch| rows.extend(batch))
            .unwrap();
        session.deliver_from(
            id(WEB_KEYWORDS),
            Delivery::Batch {
                generation,
                items: rows,
            },
        );
        let completion = session.intent(Intent::Complete).unwrap();
        assert!(completion.contains(&Effect::ReplaceQuery("g ".into())));
        assert_eq!(session.query(), "g ");

        let effects = session.set_query("gh dupin & Rust");
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search { generation, .. } => Some(*generation),
                _ => None,
            })
            .unwrap();
        let mut rows = Vec::new();
        web.search(&Request::detached("gh dupin & Rust", 1), &mut |batch| {
            rows.extend(batch)
        })
        .unwrap();
        session.deliver_from(
            id(WEB_KEYWORDS),
            Delivery::Batch {
                generation,
                items: rows,
            },
        );
        let activation = session.intent(Intent::Open).unwrap();
        assert!(activation.contains(&Effect::Run(Action::OpenUrl(
            "https://github.com/search?q=dupin%20%26%20Rust".into(),
        ))));
        assert!(!activation.contains(&Effect::SaveHistory));
    }

    #[test]
    fn invalid_dictionary_keywords_are_skipped_with_a_notice() {
        let config = Config {
            dictionary_keywords: vec!["in".into(), "".into(), "look up".into()],
            ..Config::default()
        };

        let configured = configured(&config).unwrap();
        let dictionary = id(DICTIONARY);
        assert_eq!(configured.registry.keyword("in"), Some(&id(CONTENTS)));
        assert_eq!(configured.registry.keyword("look up"), Some(&dictionary));
        let notice = configured.notice.unwrap();
        assert!(notice.contains("dictionary_keywords"));
        assert!(notice.contains("in"));
        assert!(notice.contains("must not be empty"));
    }

    #[test]
    fn qualifier_feedback_matches_only_the_file_commands() {
        let raw = "kind:pdf report";
        for command in [FILES, CONTENTS] {
            let ranges = query_qualifier_ranges(&id(command), raw);
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0], 0..8);
        }
        assert!(query_qualifier_ranges(&id(APPS), raw).is_empty());
        assert!(query_problem(&id(APPS), "kind:").is_none());
        assert!(query_problem(&id(FILES), "kind:").is_some());
        assert!(query_problem(&id(FILES), "kind:pdf").is_none());
        assert_eq!(
            query_problem(&id(CONTENTS), "kind:pdf"),
            Some("Type a word to search file contents.")
        );
        assert!(query_problem(&id(CONTENTS), raw).is_none());
        assert!(query_problem(&id(CONTENTS), "").is_none());
    }

    #[test]
    fn only_name_search_oversamples_with_a_bounded_candidate_budget() {
        for requested in [0, 2, 4000, usize::MAX] {
            let config = Config {
                file_limit: requested,
                ..Config::default()
            };
            let configured = configured(&config).unwrap();
            let session =
                dupin_core::Session::new(dupin_core::History::default(), configured.registry).unwrap();
            let names = session.provider_spec(&id(FILES)).unwrap();
            let contents = session.provider_spec(&id(CONTENTS)).unwrap();
            let shown = requested.min(MAX_FILE_CANDIDATES);
            assert_eq!(names.result_limit, shown);
            assert_eq!(
                names.candidate_limit(),
                shown.saturating_mul(4).min(MAX_FILE_CANDIDATES)
            );
            assert_eq!(contents.result_limit, shown);
            assert_eq!(contents.candidate_limit(), shown);
            let apps = session.provider_spec(&id(APPS)).unwrap();
            assert_eq!(apps.candidate_limit(), apps.result_limit);
        }
    }
}
