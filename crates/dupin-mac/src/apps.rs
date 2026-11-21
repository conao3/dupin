//! Installed apps: `.app` bundles in the standard folders, fuzzy-matched by name.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dupin_core::fuzzy;
use dupin_core::{Item, ItemKind, Provider, Request};

/// Rescan when the index is older than this, so a newly installed app turns up without a
/// file-system watcher. A scan of the standard folders takes a few milliseconds.
const STALE_AFTER: Duration = Duration::from_secs(10);
/// How deep to look for bundles below a root: `/Applications/Utilities/X.app` is depth 2.
const MAX_DEPTH: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub name: String,
    pub path: PathBuf,
    /// The bundle behind a symlink (Safari lives in a Cryptex and /Applications links to
    /// it): Finder draws an alias badge on the link's icon but not on the bundle's.
    pub icon: PathBuf,
}

pub struct AppsProvider {
    roots: Vec<PathBuf>,
    index: Mutex<Option<(Instant, Vec<App>)>>,
}

impl AppsProvider {
    /// `/Applications`, `/System/Applications` and `~/Applications`, with their subfolders
    /// (`Utilities` among them).
    pub fn standard() -> Self {
        let mut roots = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(Path::new(&home).join("Applications"));
        }
        Self::with_roots(roots)
    }

    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        AppsProvider {
            roots,
            index: Mutex::new(None),
        }
    }

    /// Scan now if the index is missing or stale, so the first search after launch or after
    /// a while doesn't pay for it.
    pub fn warm(&self) {
        drop(self.apps());
    }

    fn apps(&self) -> Vec<App> {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        match &*index {
            Some((at, apps)) if at.elapsed() < STALE_AFTER => apps.clone(),
            _ => {
                let apps = scan(&self.roots);
                *index = Some((Instant::now(), apps.clone()));
                apps
            }
        }
    }
}

impl Provider for AppsProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) {
        emit(rank(&self.apps(), &request.query, request.limit));
    }
}

/// Best matches first; equal scores keep the shorter, then alphabetical, name.
pub fn rank(apps: &[App], query: &str, limit: usize) -> Vec<Item> {
    let mut hits: Vec<(fuzzy::Match, &App)> = apps
        .iter()
        .filter_map(|app| fuzzy::score(query, &app.name).map(|m| (m, app)))
        .collect();
    hits.sort_by(|(a, x), (b, y)| b.score.cmp(&a.score).then_with(|| x.name.cmp(&y.name)));
    hits.truncate(limit);
    hits.into_iter()
        .map(|(m, app)| {
            Item::path(
                ItemKind::Application,
                app.path.clone(),
                app.name.clone(),
                m.positions,
            )
        })
        .collect()
}

/// Every `.app` under `roots`, one entry per name: the first root wins, so an app in
/// `/Applications` hides a same-named copy further down the list.
pub fn scan(roots: &[PathBuf]) -> Vec<App> {
    let mut apps = Vec::new();
    for root in roots {
        walk(root, 0, &mut apps);
    }
    let mut seen = std::collections::HashSet::new();
    apps.retain(|a: &App| seen.insert(a.name.clone()));
    apps
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<App>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        // Bundles are directories; follow symlinks too (Xcode-select style links into /Applications).
        let is_dir = kind.is_dir() || (kind.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }
        if path.extension().is_some_and(|e| e == "app") {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                let icon = if kind.is_symlink() {
                    fs::canonicalize(&path).unwrap_or_else(|_| path.clone())
                } else {
                    path.clone()
                };
                out.push(App {
                    name: name.to_owned(),
                    path,
                    icon,
                });
            }
        } else if depth + 1 < MAX_DEPTH {
            walk(&path, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dupin-apps-{}-{}", std::process::id(), paths.len()));
        let _ = fs::remove_dir_all(&root);
        for p in paths {
            fs::create_dir_all(root.join(p)).unwrap();
        }
        root
    }

    #[test]
    fn finds_bundles_one_folder_deep_but_not_inside_bundles() {
        let root = tree(&[
            "Safari.app/Contents/MacOS",
            "Utilities/Terminal.app",
            "Utilities/Deeper/Hidden.app",
            "Xcode.app/Contents/Applications/Instruments.app",
            "NotAnApp",
        ]);
        let names: Vec<_> = scan(std::slice::from_ref(&root))
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["Safari", "Terminal", "Xcode"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_linked_bundle_takes_its_icon_from_the_target() {
        let root = tree(&["Real/Linked.app/Contents"]);
        let apps_dir = root.join("Apps");
        fs::create_dir_all(&apps_dir).unwrap();
        std::os::unix::fs::symlink(root.join("Real/Linked.app"), apps_dir.join("Linked.app")).unwrap();
        let apps = scan(std::slice::from_ref(&apps_dir));
        assert_eq!(apps[0].path, apps_dir.join("Linked.app"));
        assert_eq!(
            apps[0].icon,
            fs::canonicalize(root.join("Real/Linked.app")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ranks_by_fuzzy_score() {
        let apps: Vec<App> = ["Safari Technology Preview", "Safari", "Hush Safe", "Notes"]
            .iter()
            .map(|n| {
                let path = PathBuf::from(format!("/Applications/{n}.app"));
                App {
                    name: (*n).into(),
                    icon: path.clone(),
                    path,
                }
            })
            .collect();
        let titles: Vec<_> = rank(&apps, "saf", 10).into_iter().map(|i| i.title).collect();
        assert_eq!(titles, ["Safari", "Safari Technology Preview", "Hush Safe"]);
        assert_eq!(rank(&apps, "saf", 1).len(), 1);
    }
}
