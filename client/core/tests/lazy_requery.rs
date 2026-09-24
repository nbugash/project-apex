//! After an invalidation, zero listings until the developer navigates (FR-017, SC-012a).
//!
//! The requirement is a count of requests, not a description of intent, so what is asserted is
//! that the use case cannot issue one: it holds a cache and no provider. A burst of listings at
//! the moment a link has just proved unreliable is the worst time to issue one.

mod common;

use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::use_cases::apply_file_event::ApplyFileEvent;
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

#[test]
fn an_invalidation_reads_nothing() {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src", "/src/a.rs", "/src/b.rs"]);

    let report = ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);

    // One marking, and nothing else happened at all.
    assert_eq!(report.applied, 1);
    assert_eq!(report.rows_renamed, 0);
    assert_eq!(report.marked_unproven, 0);
}

#[test]
fn the_rows_are_still_there_to_be_navigated_to() {
    // Lazy means later, not never. A tree that forgot its rows would make "re-read when they
    // navigate" impossible, because there would be nothing to navigate.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src", "/src/a.rs"]);
    ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);

    let children = cache
        .list_children(&ws(), &RelPath::parse("/src").expect("path"))
        .expect("a stale listing is still readable");
    assert_eq!(children.len(), 1);
}

#[test]
fn settling_the_doubt_is_the_hash_comparisons_job() {
    // A-UNPROVEN. `clear_unproven` exists so the flag is settled by the comparison that already
    // runs, rather than by a second mechanism that goes and asks.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src/a.rs"]);
    let id = cache
        .file_id(&ws(), &RelPath::parse("/src/a.rs").expect("path"))
        .expect("lookup")
        .expect("a row");
    assert!(cache.clear_unproven(&id).is_ok());
}
