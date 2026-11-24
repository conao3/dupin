//! The display under the pointer, read from the X server: Rune reports displays only at
//! startup, and placing the panel at a display's center needs the current one.

use std::ffi::c_void;
use std::ptr::NonNull;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

/// Why the panel couldn't open on the display the settings ask for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DisplayNotice {
    AccessibilityRequired,
    FocusedWindowUnavailable,
}

/// Linux panels have no AppKit view, so none is the key window.
///
/// # Safety
///
/// Nothing is dereferenced; the signature matches macOS's.
pub unsafe fn is_key_window(_view: NonNull<c_void>) -> bool {
    false
}

/// The height in points of the display the pointer is on, where Rune's
/// `DisplayTarget::UnderPointer` places the panel. None without an X server (a Wayland
/// session) or when no monitor holds the pointer.
pub fn pointer_display_height() -> Option<f64> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let pointer = conn.query_pointer(root).ok()?.reply().ok()?;
    let (x, y) = (i32::from(pointer.root_x), i32::from(pointer.root_y));
    let monitors = conn.randr_get_monitors(root, true).ok()?.reply().ok()?;
    let height = monitors.monitors.iter().find_map(|m| {
        let (left, top) = (i32::from(m.x), i32::from(m.y));
        let inside =
            (left..left + i32::from(m.width)).contains(&x) && (top..top + i32::from(m.height)).contains(&y);
        inside.then_some(f64::from(m.height))
    })?;
    Some(height / scale(&conn, root))
}

/// The scale winit gives X11 windows: `WINIT_X11_SCALE_FACTOR` when it is a number, else
/// `Xft.dpi` over 96, else 1.
fn scale(conn: &impl Connection, root: u32) -> f64 {
    if let Some(factor) = std::env::var("WINIT_X11_SCALE_FACTOR")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|f| *f > 0.0)
    {
        return factor;
    }
    conn.get_property(
        false,
        root,
        AtomEnum::RESOURCE_MANAGER,
        AtomEnum::STRING,
        0,
        1 << 16,
    )
    .ok()
    .and_then(|cookie| cookie.reply().ok())
    .and_then(|reply| xft_dpi(&String::from_utf8_lossy(&reply.value)))
    .map_or(1.0, |dpi| dpi / 96.0)
}

fn xft_dpi(resources: &str) -> Option<f64> {
    resources.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "Xft.dpi")
            .then(|| value.trim().parse().ok())
            .flatten()
            .filter(|d: &f64| *d > 0.0)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_xft_dpi_from_the_resource_database() {
        assert_eq!(super::xft_dpi("Xft.antialias:\t1\nXft.dpi:\t192\n"), Some(192.0));
        assert_eq!(super::xft_dpi("Xft.antialias:\t1\n"), None);
    }
}
