//! Unproven content stays servable while disconnected (FR-019b, SC-006b).
//!
//! The copy the developer can still read is the whole reason for keeping it. Throwing it away
//! for a change they may never open would trade a possibly-stale file for no file at all.

mod common;

use apex_shell::application::use_cases::apply_file_event::{ApplyFileEvent, Change, FileEvent};
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

#[test]
fn the_blob_is_still_there_after_an_event_says_it_changed() {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src/a.rs"]);
    ApplyFileEvent::new(&cache).apply(
        &ws(),
        &[FileEvent {
            path: RelPath::parse("/src/a.rs").expect("path"),
            change: Change::Modified {
                is_directory: false,
                size: 1,
                modified: 1,
            },
        }],
    );
    assert!(
        common::fake_cache::paths(&cache, &ws()).contains(&"/src/a.rs".to_string()),
        "marked, not removed"
    );
}

#[test]
fn unproven_is_not_a_validity_state() {
    // A-UNPROVEN, expressed as a fact about the types: `Validity` has one constructor and it
    // takes two hashes. This test exists so that adding a third variant fails something.
    use apex_shell::domain::cache::Validity;
    use apex_shell::domain::workspace::Sha256;
    let same = Sha256::of(b"x");
    for value in [
        Validity::compare(Some(&same), Some(&same)),
        Validity::compare(Some(&same), Some(&Sha256::of(b"y"))),
        Validity::compare(None, Some(&same)),
    ] {
        let rendered = format!("{value:?}").to_lowercase();
        assert!(
            !rendered.contains("unproven"),
            "unproven is a flag beside validity, never one of its states: {rendered}"
        );
    }
}
