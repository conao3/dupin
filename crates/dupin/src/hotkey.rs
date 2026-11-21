//! The global shortcut that toggles the panel. A failed registration is never silent: it is
//! logged, posted as a notification when the app can post them, and shown in the panel,
//! which opens to show it, since an accessory app has no other surface.
//!
//! What can't be detected: another app holding the chord through a non-exclusive Carbon
//! registration (Raycast's, for one). Both registrations succeed and macOS delivers the key
//! to one of them, so the README asks users to unassign ⌘Space elsewhere first.

use rune::{AppHandle, Shortcut, ShortcutError};

use crate::ui::Ui;

pub fn register(handle: &AppHandle<Ui>, accelerator: &str) {
    let shortcut = match accelerator.parse::<Shortcut>() {
        Ok(shortcut) => shortcut,
        Err(e) => return report(handle, accelerator, &e.to_string()),
    };
    // Carbon only refuses a chord another app grabbed exclusively. macOS's own shortcuts
    // (Spotlight's ⌘Space) take the key first without making registration fail, so ask who
    // holds it beforehand. Register anyway: freeing the chord later then works without a restart.
    if let Some(owner) = handle.shortcut_owner(&shortcut) {
        report(handle, accelerator, &format!("{owner} already uses it"));
    }
    if let Err(e) = handle.register_shortcut(shortcut.clone(), |s: &mut Ui, tx| s.toggle(tx)) {
        report(handle, accelerator, &why(handle, &shortcut, &e));
    }
}

fn report(handle: &AppHandle<Ui>, accelerator: &str, reason: &str) {
    let settings =
        crate::settings::path().map_or_else(|| "config.toml".to_owned(), |p| p.display().to_string());
    let message = format!(
        "Couldn't register the {} hotkey: {reason}. Free it in the app that holds it, or set another `hotkey` in {settings}.",
        keycaps(accelerator)
    );
    crate::log::warn(&message);
    notify(&message);
    handle.update(move |s, tx| s.show_notice(message, tx));
}

fn why(handle: &AppHandle<Ui>, shortcut: &Shortcut, error: &ShortcutError) -> String {
    match (error, handle.shortcut_owner(shortcut)) {
        (ShortcutError::Conflict, Some(owner)) => format!("{owner} already uses it"),
        (ShortcutError::Conflict, None) => "another app already uses it".to_owned(),
        (e, _) => e.to_string(),
    }
}

fn notify(message: &str) {
    if rune_notify::available() {
        rune_notify::post(
            &rune_notify::Notification::new("hotkey", "Dupin hotkey unavailable").body(message),
        );
    }
}

/// `cmd+space` as macOS writes it: ⌘Space.
pub fn keycaps(accelerator: &str) -> String {
    let mut out = String::new();
    for part in accelerator.split('+').map(str::trim) {
        out.push_str(match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "mod" => "⌘",
            "alt" | "option" | "opt" => "⌥",
            "ctrl" | "control" => "⌃",
            "shift" => "⇧",
            "space" => "Space",
            _ => {
                out.push_str(&part.to_uppercase());
                continue;
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_modifiers_as_macos_glyphs() {
        assert_eq!(super::keycaps("cmd+space"), "⌘Space");
        assert_eq!(super::keycaps("ctrl+alt+k"), "⌃⌥K");
    }
}
