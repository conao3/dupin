//! Running actions through NSWorkspace.

use std::path::Path;

use dupin_core::Action;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSArray, NSString, NSURL};

/// Carry out `action`. Returns false when the system refused (a vanished file, say).
pub fn run(action: &Action) -> bool {
    match action {
        Action::Open(path) => open(path),
        Action::Reveal(path) => {
            reveal(path);
            true
        }
    }
}

/// Open with the default handler: launches an app bundle, opens a document or folder.
pub fn open(path: &Path) -> bool {
    NSWorkspace::sharedWorkspace().openURL(&file_url(path))
}

/// Show in Finder with the item selected.
pub fn reveal(path: &Path) {
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[file_url(path)]));
}

fn file_url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}
