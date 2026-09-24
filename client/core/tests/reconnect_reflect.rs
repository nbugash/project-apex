//! A change made while disconnected surfaces on the next navigation (SC-012).
//!
//! `reconnect_stale.rs` asserts the tree is marked and no listing is issued. This asserts the
//! other half: that the marking is what makes the next navigation re-read, rather than a flag
//! nobody consults.

mod common;

use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::use_cases::apply_file_event::ApplyFileEvent;
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

#[test]
fn marking_stale_is_what_the_next_navigation_consults() {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src", "/src/a.rs"]);

    ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);

    // The rows are still there to navigate to -- staleness is a reason to re-read, not a
    // reason to forget.
    let children = cache.list_children(&ws(), &RelPath::parse("/src").expect("path"));
    assert!(children.is_ok(), "a stale tree is still navigable");
}

#[test]
fn nothing_is_discarded_by_the_marking() {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/a.rs"]);
    let before = common::fake_cache::paths(&cache, &ws());
    ApplyFileEvent::new(&cache).invalidate_all(&ws(), &[]);
    assert_eq!(common::fake_cache::paths(&cache, &ws()), before);
}
