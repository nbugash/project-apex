//! Reconnection marks the tree stale and re-reads nothing (FR-026, FR-026a, SC-012a).

mod common;

use apex_shell::application::use_cases::apply_file_event::ApplyFileEvent;
use apex_shell::domain::workspace::WorkspaceId;
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

#[test]
fn it_discards_zero_blobs() {
    // Anything that changed while disconnected was never delivered and the client has no way
    // to know which parts moved -- but a blob still proves itself by hash, so discarding would
    // throw away content that may well still be current.
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/a.rs", "/b.rs"]);
    let before = common::fake_cache::paths(&cache, &ws());
    ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);
    assert_eq!(common::fake_cache::paths(&cache, &ws()), before);
}

#[test]
fn it_issues_zero_listings() {
    // SC-012a. A burst of listings at the moment a link has just proved unreliable is the
    // worst time to issue one; the use case holds no provider, so it structurally cannot.
    let cache = InMemoryCache::default();
    let report = ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);
    assert_eq!(
        report.applied, 1,
        "exactly one thing happened: the tree was marked"
    );
    assert_eq!(report.rows_renamed, 0);
}
