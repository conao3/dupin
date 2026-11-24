//! Where Dupin keeps its files on Linux, by the XDG Base Directory Specification.

use std::path::PathBuf;

/// `$XDG_STATE_HOME/dupin`, or `~/.local/state/dupin`: the history and the log.
pub fn state() -> Option<PathBuf> {
    base("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_DATA_HOME/dupin`, or `~/.local/share/dupin`: extensions, their storage and the
/// settings overlay.
pub fn data() -> Option<PathBuf> {
    base("XDG_DATA_HOME", ".local/share")
}

/// `$var/dupin` when `var` holds an absolute path, else `~/fallback/dupin`.
fn base(var: &str, fallback: &str) -> Option<PathBuf> {
    let base = std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))?;
    Some(base.join("dupin"))
}
