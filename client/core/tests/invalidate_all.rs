//! A wholesale invalidation marks the tree stale and discards no content
//! (FR-017, FR-018, FR-023b, SC-006, SC-004a).

mod common;

use apex_shell::application::use_cases::apply_file_event::ApplyFileEvent;
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}
fn rel(p: &str) -> RelPath {
    RelPath::parse(p).expect("a path")
}

#[test]
fn it_discards_zero_cached_blobs() {
    // SC-006. Tree invalidation is not content invalidation: each blob remains valid or not on
    // its own hash terms, and a branch switch that emptied the cache would cost a developer
    // every file they had read.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/a.rs", "/b.rs", "/deep/c.rs"]);
    let before = common::fake_cache::paths(&cache, &ws());

    ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);

    assert_eq!(
        common::fake_cache::paths(&cache, &ws()),
        before,
        "every row survived"
    );
}

#[test]
fn it_marks_every_open_tab_unproven() {
    // FR-023b, SC-004a. The bulk rule discards the individual events, so without this a branch
    // switch that rewrote a file the developer has open reports nothing about it -- and FR-023
    // admits no exception for how a change arrived.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/a.rs", "/b.rs"]);
    let report = ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[rel("/a.rs"), rel("/b.rs")]);
    assert_eq!(report.marked_unproven, 2);
}

#[test]
fn it_marks_nothing_when_no_tab_is_open() {
    // FR-024 from the other side: the mechanism satisfying FR-023 must not itself interrupt
    // somebody about files they have not opened.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/a.rs"]);
    let report = ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);
    assert_eq!(report.marked_unproven, 0);
    assert_eq!(report.applied, 1, "the tree is still marked stale");
}

#[test]
fn it_is_not_a_fetch_instruction() {
    // FR-017. The client marks stale and re-queries lazily; this use case cannot fetch because
    // it holds no provider.
    let cache = InMemoryCache::default();
    let report = ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);
    assert_eq!(report.rows_renamed, 0);
    assert_eq!(report.refused, 0);
}
