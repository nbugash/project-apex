//! Cached content survives a rename (SC-008, FR-021).
//!
//! This is why FR-021 exists at all. A delete followed by a create loses the blob; a move keeps
//! it, because `file_id` is opaque and the content is keyed by it rather than by the path.

mod common;

use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}
fn rel(p: &str) -> RelPath {
    RelPath::parse(p).expect("a path")
}

#[test]
fn the_renamed_file_is_addressable_at_its_new_path() {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src/expr.rs"]);
    let before = cache.file_id(&ws(), &rel("/src/expr.rs")).expect("lookup");
    assert!(before.is_some(), "the fixture has a row");

    cache
        .rename_subtree(&ws(), &rel("/src/expr.rs"), &rel("/src/expression.rs"))
        .expect("rename");

    let after = cache
        .file_id(&ws(), &rel("/src/expression.rs"))
        .expect("lookup");
    assert_eq!(after, before, "the same file_id, so the same content");
    assert!(
        cache
            .file_id(&ws(), &rel("/src/expr.rs"))
            .expect("lookup")
            .is_none(),
        "and nothing left at the old path"
    );
}

#[test]
fn a_renamed_directorys_descendants_keep_their_identities() {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(&cache, &ws(), &["/src", "/src/deep/a.rs"]);
    let before = cache
        .file_id(&ws(), &rel("/src/deep/a.rs"))
        .expect("lookup");

    cache
        .rename_subtree(&ws(), &rel("/src"), &rel("/syntax"))
        .expect("rename");

    let after = cache
        .file_id(&ws(), &rel("/syntax/deep/a.rs"))
        .expect("lookup");
    assert_eq!(
        after, before,
        "a directory rename moves entries; it does not replace them"
    );
}
