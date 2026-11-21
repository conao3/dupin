# Dupin

A lightweight, resident launcher for Apple silicon Macs, built with [Rune](https://github.com/anomalyco/rune).

Homepage: <https://dupin.app>

Named after C. Auguste Dupin, Poe's detective: the source of the line of Mac launchers that runs Sherlock → Watson → Alfred.

## Before you start: free ⌘Space

Dupin's hotkey is **⌘Space**. macOS delivers a chord to only one app, so before running Dupin:

1. Turn off Spotlight's ⌘Space: System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → untick "Show Spotlight search".
2. Remove ⌘Space from any other app that uses it, such as Raycast (Settings → General → Raycast Hotkey) or Alfred.

If ⌘Space is still taken when Dupin starts, Dupin says so: it writes the reason to `~/Library/Logs/Dupin/dupin.log` and stderr, posts a notification when it runs as a bundled app, and opens the panel with the message. There is one case it can't detect: another app that registered the chord non-exclusively, as Raycast does. macOS accepts both registrations and gives the key to only one of them, so step 2 matters.

To use a different chord, set it in `~/.config/dupin/config.toml` (or `$XDG_CONFIG_HOME/dupin/config.toml`):

```toml
hotkey = "alt+space"   # modifiers cmd, alt, ctrl, shift, joined with + to a key
file_limit = 200       # the most files a Spotlight search shows
```

## Use

| Key | In the panel |
| --- | --- |
| ⌘Space | Show or hide the panel |
| Type | Fuzzy-search the apps in `/Applications`, `/System/Applications` and `~/Applications` (Utilities and other subfolders included) |
| Space on an empty query | Search files with Spotlight (by file and display name) |
| `in` then Space | Search inside files with Spotlight (words in the indexed text, by prefix), as Alfred's `in` |
| ⌫ on an empty query | Leave file or content search |
| ↑ ↓ | Move the selection |
| ↩ | Open the app or file |
| ⌘↩ | Reveal the file in Finder |
| Esc, or clicking elsewhere | Hide the panel |
| ⌘Q | Quit Dupin (it has no Dock icon or menu bar) |

## Build and run

Dupin builds with Nix; the toolchain comes from nixpkgs and matches Rune's `rust-toolchain.toml`.

```sh
nix build .#dupin                     # result/Applications/Dupin.app
open result/Applications/Dupin.app
nix flake check                       # clippy, tests, rustfmt and the app bundle
```

For development, `direnv allow` (or `nix develop`) gives a shell with cargo, clippy, rustfmt and rust-analyzer:

```sh
cargo run -p dupin
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p dupin-mac --example search -- in invoice      # a provider from the terminal (apps, files, in)
scripts/snap.sh                        # headless renders of scripts/headless/*.json into target/screenshots
```

`DUPIN_TRACE=1` prints, for each summon, how long the panel took from the hotkey handler to its first drawn frame.

Debug builds take Rune's headless flags (`--snap`, `--tree`, `--script`, `--realtime`, ...); see Rune's `crates/rune/README.md`, "Agent tooling".

## Releases

[tagpr](https://github.com/Songmu/tagpr) keeps a release pull request open against `main`. It bumps `VERSION`; merging it tags the merge commit and creates the GitHub release. The crates stay at version 0.0.0, so a release changes neither `Cargo.lock` nor the vendored-dependency hash in `flake.nix`.

## Layout

| Crate | What it holds |
| --- | --- |
| `crates/dupin-core` | No UI, no macOS: the `Provider` trait and `Item` model, the newest-wins `Searcher`, fuzzy matching, the launcher `Session` (modes, selection, effects) and `config.toml` |
| `crates/dupin-mac` | The apps and Spotlight (names and contents) providers and NSWorkspace actions, through objc2 |
| `crates/dupin` | The Rune app: the panel view, key mapping and hotkey; kept thin so the UI framework can change without touching the rest |

Every source of results is a `Provider` that streams batches of `Item`s on its own worker thread. Apps and files use it today; extensions (scripts that print a JSON view to stdout) are meant to use it too.

## Working around Rune

Rune is pre-release and doesn't take pull requests, so Dupin works around what it hits. These are pinned to the rev in `Cargo.toml`; check them again when bumping it.

- **`rune-text` embeds fonts from outside its crate.** It uses `include_bytes!("../../../rune/assets/Inter-*.ttf")`, a path that exists in a Git checkout of the whole repository but not once crates are vendored one by one. The Nix build symlinks the vendored `rune` crate into place (`postPatch` in `flake.nix`).
- **Two `tiny-skia-path 0.11.4` packages.** Rune vendors a patched copy as a path dependency while `resvg` keeps the published one. nixpkgs' `importCargoLock` can't vendor both, so the flake uses `fetchCargoVendor`.
- **Headless snapshots don't follow `fit_content_height`.** `--snap` renders at the window's initial size, so `scripts/snap.sh` reads the panel's height from `--tree` first.
- **The headless clock outruns real providers.** Results come from Spotlight and the file system in real time, so headless renders pass `--realtime` (listed in `--help` but not in the README).
- **Hotkey conflicts with macOS's own shortcuts register successfully.** Rune names Spotlight as the owner only after a `Conflict`, and Carbon reports none for it, so Dupin asks `shortcut_owner` before registering.
