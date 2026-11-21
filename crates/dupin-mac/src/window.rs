//! AppKit settings for the panel's own window.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSView, NSWindow, NSWindowAnimationBehavior};

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
