//! An event marks content unproven; it never marks it valid (FR-019, FR-019a, A-UNPROVEN).
//!
//! `Validity` has one constructor and it takes two hashes, so nothing but a hash comparison can
//! declare content valid. An event is not a hash. Doubt is the only thing it can cast, and the
//! flag sits beside validity rather than inside it precisely so that stays true.

mod common;

use apex_shell::application::use_cases::apply_file_event::{ApplyFileEvent, Change, FileEvent};
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}
fn rel(p: &str) -> RelPath {
    RelPath::parse(p).expect("a path")
}

fn modified(path: &str) -> FileEvent {
    FileEvent {
        path: rel(path),
        change: Change::Modified {
            is_directory: false,
            size: 9,
            modified: 3,
        },
    }
}

#[test]
fn an_event_on_a_cached_file_discards_zero_blobs() {
    // SC-006a. Discarding would throw away content for a change the developer may never open,
    // and remove the copy they can still read offline.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src/a.rs"]);
    let before = common::fake_cache::paths(&cache, &ws()).len();

    ApplyFileEvent::new(&cache).apply(&ws(), &[modified("/src/a.rs")]);

    assert_eq!(
        common::fake_cache::paths(&cache, &ws()).len(),
        before,
        "nothing was discarded"
    );
}

#[test]
fn an_event_causes_zero_extra_confirmations() {
    // SC-006a. The existing hash check does the work; the flag is a hint that it will
    // disagree, not a second mechanism that goes and asks.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src/a.rs"]);
    let report = ApplyFileEvent::new(&cache).apply(&ws(), &[modified("/src/a.rs")]);
    assert_eq!(report.marked_unproven, 1);
    assert_eq!(
        report.applied, 1,
        "one event, one application, no round trip"
    );
}

#[test]
fn an_event_for_a_path_never_fetched_fetches_nothing() {
    // FR-020. There is no path from this use case to a provider at all -- it holds a cache and
    // nothing else, which is the structural version of the requirement.
    let cache = InMemoryCache::default();
    let report = ApplyFileEvent::new(&cache).apply(&ws(), &[modified("/never/seen.rs")]);
    assert_eq!(
        report.applied, 1,
        "applied as a no-op rather than triggering a fetch"
    );
}

#[test]
fn marking_an_already_unproven_blob_changes_nothing() {
    // Obligation 17, and a spec edge case: the hash already disagrees and the file is already
    // unproven, so a second event must not cause a second fetch.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src/a.rs"]);
    let apply = ApplyFileEvent::new(&cache);
    let first = apply.apply(&ws(), &[modified("/src/a.rs")]);
    let second = apply.apply(&ws(), &[modified("/src/a.rs")]);
    assert_eq!(first.marked_unproven, second.marked_unproven, "idempotent");
    assert_eq!(second.applied, 1);
}

#[test]
fn a_deleted_event_marks_the_region_stale_rather_than_discarding_content() {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src/a.rs"]);
    let report = ApplyFileEvent::new(&cache).apply(
        &ws(),
        &[FileEvent {
            path: rel("/src/a.rs"),
            change: Change::Deleted,
        }],
    );
    assert_eq!(report.applied, 1);
    assert_eq!(
        report.marked_unproven, 0,
        "a deletion is not a doubt about content"
    );
}
