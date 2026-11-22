//! AppKit settings for the panel's own window.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSScreen, NSView, NSWindow, NSWindowAnimationBehavior};

/// Make the window holding `ns_view` appear and disappear at once. AppKit animates a panel's
/// ordering in by default, which held Dupin's panel off screen for about 265 ms after its
/// frame was ready; a launcher has to be there the moment its hotkey lands.
///
/// # Safety
///
/// `ns_view` must point to a live `NSView`, as a window's raw AppKit handle does while the
/// window exists.
pub unsafe fn appear_instantly(ns_view: NonNull<c_void>) -> bool {
    let Some(_main) = MainThreadMarker::new() else {
        return false;
    };
    // SAFETY: The caller guarantees a live NSView; we only borrow it for this call.
    let view: &NSView = unsafe { ns_view.cast().as_ref() };
    let Some(window): Option<objc2::rc::Retained<NSWindow>> = view.window() else {
        return false;
    };
    window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    true
}

/// Geometry for one display, in AppKit's global point coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayGeometry {
    /// The Core Graphics display identifier associated with the screen.
    pub native_id: u32,
    /// The full display frame as `[x, y, width, height]` in AppKit points.
    pub frame: [f64; 4],
    /// The usable display frame as `[x, y, width, height]` in AppKit points.
    pub visible_frame: [f64; 4],
    /// Whether Active selection fell back to the primary display because its focused window
    /// could not be read.
    pub active_unavailable: bool,
}

/// Snapshot the selected display and its usable area. Active selection reads the focused window
/// through Accessibility only when permission is already granted; it never prompts.
pub fn display_geometry(selection: dupin_core::DisplaySelection) -> Option<DisplayGeometry> {
    let main = MainThreadMarker::new()?;
    let screens = NSScreen::screens(main);
    let snapshots: Vec<_> = screens
        .iter()
        .map(|screen| ScreenSnapshot {
            native_id: native_id(&screen),
            frame: rect(screen.frame()),
            visible_frame: rect(screen.visibleFrame()),
        })
        .collect();
    let primary_height = snapshots.first()?.frame[3];
    let pointer = NSEvent::mouseLocation();
    let pointer = (pointer.x, pointer.y);
    let active = if selection == dupin_core::DisplaySelection::Active {
        active_window_frame(primary_height)
    } else {
        None
    };
    let screen_frames: Vec<_> = snapshots.iter().map(|screen| screen.frame).collect();
    let (index, active_unavailable) = select_screen(&screen_frames, pointer, active, selection)?;
    let screen = snapshots.get(index)?;
    Some(DisplayGeometry {
        native_id: screen.native_id?,
        frame: screen.frame,
        visible_frame: screen.visible_frame,
        active_unavailable,
    })
}

struct ScreenSnapshot {
    native_id: Option<u32>,
    frame: [f64; 4],
    visible_frame: [f64; 4],
}

fn native_id(screen: &NSScreen) -> Option<u32> {
    use objc2_foundation::{NSNumber, NSString};

    let key = NSString::from_str("NSScreenNumber");
    screen
        .deviceDescription()
        .objectForKey(&key)?
        .downcast::<NSNumber>()
        .ok()
        .map(|number| number.as_u32())
}

fn rect(rect: objc2_foundation::NSRect) -> [f64; 4] {
    [rect.origin.x, rect.origin.y, rect.size.width, rect.size.height]
}

fn select_screen(
    screens: &[[f64; 4]],
    pointer: (f64, f64),
    active_frame: Option<[f64; 4]>,
    selection: dupin_core::DisplaySelection,
) -> Option<(usize, bool)> {
    if screens.is_empty() {
        return None;
    }
    let selected = match selection {
        dupin_core::DisplaySelection::Mouse => screens
            .iter()
            .position(|screen| contains(*screen, pointer))
            .map(|index| (index, false))
            .unwrap_or((0, false)),
        dupin_core::DisplaySelection::Main => (0, false),
        dupin_core::DisplaySelection::Active => active_frame
            .and_then(|frame| screen_with_largest_overlap(screens, frame))
            .map(|index| (index, false))
            .unwrap_or((0, true)),
    };
    Some(selected)
}

fn contains(frame: [f64; 4], point: (f64, f64)) -> bool {
    point.0 >= frame[0]
        && point.0 < frame[0] + frame[2]
        && point.1 > frame[1]
        && point.1 <= frame[1] + frame[3]
}

fn screen_with_largest_overlap(screens: &[[f64; 4]], frame: [f64; 4]) -> Option<usize> {
    if !usable_frame(frame) {
        return None;
    }
    screens
        .iter()
        .enumerate()
        .filter(|(_, screen)| usable_frame(**screen))
        .map(|(index, screen)| (index, overlap(*screen, frame)))
        .filter(|(_, area)| *area > 0.0)
        .fold(None, |best, candidate| match best {
            Some((_, area)) if area >= candidate.1 => best,
            _ => Some(candidate),
        })
        .map(|(index, _)| index)
}

fn usable_frame(frame: [f64; 4]) -> bool {
    frame.into_iter().all(f64::is_finite) && frame[2] > 0.0 && frame[3] > 0.0
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let width = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let height = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    width.max(0.0) * height.max(0.0)
}

fn ax_to_appkit_frame(position: AxPoint, size: AxSize, primary_height: f64) -> [f64; 4] {
    [
        position.x,
        primary_height - position.y - size.height,
        size.width,
        size.height,
    ]
}

#[repr(C)]
#[derive(Default)]
struct AxPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Default)]
struct AxSize {
    width: f64,
    height: f64,
}

fn active_window_frame(primary_height: f64) -> Option<[f64; 4]> {
    if !accessibility::is_trusted() {
        return None;
    }
    let system = accessibility::system_wide()?;
    let focused_app = accessibility::attribute(&system, "AXFocusedApplication")?;
    let focused_window = accessibility::attribute(&focused_app, "AXFocusedWindow")?;
    let position = accessibility::attribute(&focused_window, "AXPosition")?;
    let size = accessibility::attribute(&focused_window, "AXSize")?;
    let position = accessibility::point(&position)?;
    let size = accessibility::size(&size)?;
    Some(ax_to_appkit_frame(position, size, primary_height))
}

mod accessibility {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_foundation::{CFRetained, CFString, CFType};

    type ElementRef = *const c_void;
    type TypeRef = *const c_void;
    type Error = i32;
    const MESSAGING_TIMEOUT_SECONDS: f32 = 0.1;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXUIElementCreateSystemWide() -> ElementRef;
        fn AXUIElementSetMessagingTimeout(element: ElementRef, timeout: f32) -> Error;
        fn AXUIElementCopyAttributeValue(
            element: ElementRef,
            attribute: *const CFString,
            value: *mut TypeRef,
        ) -> Error;
        fn AXValueGetValue(value: TypeRef, value_type: u32, value_ptr: *mut c_void) -> u8;
    }

    pub(super) fn is_trusted() -> bool {
        // SAFETY: This query only reports the current process's existing Accessibility grant.
        unsafe { AXIsProcessTrusted() != 0 }
    }

    pub(super) fn system_wide() -> Option<CFRetained<CFType>> {
        // SAFETY: AXUIElementCreateSystemWide follows the Create rule and returns a CF object.
        let element = unsafe { AXUIElementCreateSystemWide() };
        let element = retained(element)?;
        set_messaging_timeout(&element).then_some(element)
    }

    pub(super) fn attribute(element: &CFRetained<CFType>, name: &str) -> Option<CFRetained<CFType>> {
        if !set_messaging_timeout(element) {
            return None;
        }
        let name = CFString::from_str(name);
        let mut value = std::ptr::null();
        // SAFETY: Both pointers are live CF objects, the output pointer is valid, and Copy
        // returns an owned reference on success.
        let error = unsafe {
            AXUIElementCopyAttributeValue(
                CFRetained::as_ptr(element).as_ptr().cast(),
                CFRetained::as_ptr(&name).as_ptr(),
                &mut value,
            )
        };
        if error != 0 {
            return None;
        }
        retained(value)
    }

    fn set_messaging_timeout(element: &CFRetained<CFType>) -> bool {
        // SAFETY: `element` is a live AXUIElement CF object returned by a Create/Copy API.
        // The finite timeout bounds calls to an unresponsive external application.
        unsafe {
            AXUIElementSetMessagingTimeout(
                CFRetained::as_ptr(element).as_ptr().cast(),
                MESSAGING_TIMEOUT_SECONDS,
            ) == 0
        }
    }

    pub(super) fn point(value: &CFRetained<CFType>) -> Option<super::AxPoint> {
        let mut point = super::AxPoint::default();
        // SAFETY: AXPosition is specified to contain an AXValue wrapping a CGPoint.
        let success = unsafe {
            AXValueGetValue(
                CFRetained::as_ptr(value).as_ptr().cast(),
                1,
                (&mut point as *mut super::AxPoint).cast(),
            )
        };
        (success != 0).then_some(point)
    }

    pub(super) fn size(value: &CFRetained<CFType>) -> Option<super::AxSize> {
        let mut size = super::AxSize::default();
        // SAFETY: AXSize is specified to contain an AXValue wrapping a CGSize.
        let success = unsafe {
            AXValueGetValue(
                CFRetained::as_ptr(value).as_ptr().cast(),
                2,
                (&mut size as *mut super::AxSize).cast(),
            )
        };
        (success != 0).then_some(size)
    }

    fn retained(value: TypeRef) -> Option<CFRetained<CFType>> {
        let value = NonNull::new(value.cast_mut().cast::<CFType>())?;
        // SAFETY: AX Create/Copy functions return an owned +1 CF reference.
        Some(unsafe { CFRetained::from_raw(value) })
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
