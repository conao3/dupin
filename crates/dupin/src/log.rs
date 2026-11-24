//! Messages a user may need to find later: printed to stderr and appended to
//! `~/Library/Logs/Dupin/dupin.log`, where Console.app shows them (on Linux,
//! `$XDG_STATE_HOME/dupin/dupin.log`). A bundled app's stderr goes
//! nowhere, so the file is what makes a startup problem findable.
//!
//! Only for rare events (startup problems), never from the frame loop.

use std::io::Write;
#[cfg(target_os = "macos")]
use std::path::PathBuf;

pub fn warn(message: &str) {
    eprintln!("dupin: {message}");
    #[cfg(target_os = "macos")]
    let dir = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Logs/Dupin"));
    #[cfg(target_os = "linux")]
    let dir = crate::platform::dirs::state();
    let Some(dir) = dir else {
        return;
    };
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("dupin.log"))?;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        writeln!(file, "{secs} {message}")
    };
    if let Err(e) = write() {
        eprintln!("dupin: couldn't write the log file: {e}");
    }
}
