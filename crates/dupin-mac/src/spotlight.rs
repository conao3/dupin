//! File search through the Spotlight index (`NSMetadataQuery`), matching file and display
//! names and streaming results in the order Spotlight finds them.

use std::path::PathBuf;
use std::time::Duration;

use dupin_core::{Item, ItemKind, Provider, Request};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSArray, NSDate, NSDefaultRunLoopMode, NSMetadataItem, NSMetadataItemPathKey, NSMetadataQuery,
    NSMetadataQueryLocalComputerScope, NSPredicate, NSRunLoop, NSString,
};

/// How long the worker's run loop turns between looks at the query: short enough that a
/// cancel lands within a frame or two and the first results show at once, long enough not to spin.
const SLICE: Duration = Duration::from_millis(16);

/// Spotlight's file search. Its `search` must run on a thread it can turn the run loop of,
/// which a [`Searcher`](dupin_core::Searcher)'s dedicated worker is.
#[derive(Default)]
pub struct SpotlightProvider;

impl Provider for SpotlightProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) {
        autoreleasepool(|_| run_query(request, emit));
    }
}

fn run_query(request: &Request, emit: &mut dyn FnMut(Vec<Item>)) {
    let Some(predicate) = predicate(&request.query) else {
        return;
    };
    let query = NSMetadataQuery::new();
    query.setPredicate(Some(&predicate));
    // SAFETY: The scope constant is an immutable NSString AppKit exports; the property takes an
    // array of such scope strings.
    unsafe {
        let scopes: Retained<NSArray> =
            NSArray::from_slice(&[NSMetadataQueryLocalComputerScope as &AnyObject]);
        query.setSearchScopes(&scopes);
    }
    query.setNotificationBatchingInterval(SLICE.as_secs_f64());
    if !query.startQuery() {
        return;
    }

    let run_loop = NSRunLoop::currentRunLoop();
    let mut seen = 0;
    loop {
        let until = NSDate::dateWithTimeIntervalSinceNow(SLICE.as_secs_f64());
        // SAFETY: The default mode constant is a valid run loop mode.
        if !run_loop.runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &until) {
            // No input sources yet: nothing would wake the loop, so don't spin.
            std::thread::sleep(SLICE);
        }
        if request.is_cancelled() {
            break;
        }
        // Freeze the result list while reading it, so indices stay put.
        query.disableUpdates();
        let gathering = query.isGathering();
        let count = query.resultCount().min(request.limit);
        let batch: Vec<Item> = (seen..count)
            .filter_map(|i| item(&query.resultAtIndex(i)))
            .collect();
        query.enableUpdates();
        seen = count;
        if !batch.is_empty() {
            emit(batch);
        }
        if !gathering || seen >= request.limit {
            break;
        }
    }
    query.stopQuery();
}

fn item(result: &AnyObject) -> Option<Item> {
    let result: &NSMetadataItem = result.downcast_ref()?;
    // SAFETY: The path key is a valid attribute name constant.
    let path = result.valueForAttribute(unsafe { NSMetadataItemPathKey })?;
    let path: Retained<NSString> = path.downcast().ok()?;
    let path = PathBuf::from(path.to_string());
    let title = path.file_name()?.to_string_lossy().into_owned();
    Some(Item::path(ItemKind::File, path, title, Vec::new()))
}

fn predicate(query: &str) -> Option<Retained<NSPredicate>> {
    NSPredicate::predicateFromMetadataQueryString(&NSString::from_str(&query_string(query)?))
}

/// The Spotlight query string for `query`: every word must appear in the file name or the
/// display name, ignoring case and diacritics. None for a query with no words.
pub fn query_string(query: &str) -> Option<String> {
    let clauses: Vec<String> = query
        .split_whitespace()
        .map(|word| {
            let w = escape(word);
            format!("(kMDItemFSName == \"*{w}*\"cd || kMDItemDisplayName == \"*{w}*\"cd)")
        })
        .collect();
    (!clauses.is_empty()).then(|| clauses.join(" && "))
}

/// Inside a quoted Spotlight value, `*` is a wildcard and `\` and `"` need escaping; a typed
/// `*` should match itself.
fn escape(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if matches!(c, '\\' | '"' | '*' | '\'') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_must_match_a_name() {
        assert_eq!(
            query_string("tax  2026").unwrap(),
            "(kMDItemFSName == \"*tax*\"cd || kMDItemDisplayName == \"*tax*\"cd) && \
             (kMDItemFSName == \"*2026*\"cd || kMDItemDisplayName == \"*2026*\"cd)"
        );
        assert_eq!(query_string("   "), None);
    }

    #[test]
    fn escapes_quotes_wildcards_and_backslashes() {
        assert_eq!(escape(r#"a"b*c\d"#), r#"a\"b\*c\\d"#);
    }

    #[test]
    fn the_query_string_parses_as_a_predicate() {
        assert!(predicate(r#"weird "name" *.txt"#).is_some());
    }
}
