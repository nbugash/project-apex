//! The test that would have caught the gap in §5.2.
//!
//! `files_fts` is declared `content='files'`, which makes it external-content: SQLite stores the
//! index and reads values back from `files`, and does **not** keep it in step. Without triggers
//! the table is created empty and stays empty — so every offline path search returns nothing,
//! quickly, with no error. That is the worst available failure, and it is invisible to any test
//! that asserts only "the search succeeded".

mod common;

use apex_shell::adapters::outbound::sqlite::{schema, SqliteWorkspaceCache};
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::domain::workspace::{
    EntryKind, FsEntry, Location, RelPath, Workspace, WorkspaceId,
};

fn cache() -> (SqliteWorkspaceCache, WorkspaceId) {
    let c = SqliteWorkspaceCache::in_memory().expect("open");
    c.migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("v1");
    let ws = WorkspaceId("w1".into());
    c.register(
        &Workspace {
            id: ws.clone(),
            name: "r".into(),
            location: Location::Remote {
                host: "h".into(),
                base: "/b".into(),
            },
            last_opened_at: 0,
        },
        0,
    )
    .unwrap();
    (c, ws)
}

fn entry(name: &str) -> FsEntry {
    FsEntry {
        name: name.into(),
        kind: EntryKind::File,
        size: 1,
        modified: 0,
    }
}

#[test]
fn an_inserted_file_is_findable() {
    let (c, ws) = cache();
    c.put_listing(&ws, &RelPath::root(), &[entry("main.rs")])
        .unwrap();
    let hits = c.search_paths(&ws, "main", 10).unwrap();
    assert_eq!(
        hits.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        vec!["/main.rs"],
        "an insert must reach the index, or offline search is blind to everything ever listed"
    );
}

#[test]
fn a_renamed_file_is_findable_under_its_new_name_and_not_its_old_one() {
    let (c, ws) = cache();
    c.put_listing(&ws, &RelPath::root(), &[entry("main.rs")])
        .unwrap();
    let id = c
        .file_id(&ws, &RelPath::parse("/main.rs").unwrap())
        .unwrap()
        .unwrap();

    c.rename(&id, &RelPath::parse("/lib.rs").unwrap()).unwrap();

    assert!(
        c.search_paths(&ws, "lib", 10)
            .unwrap()
            .iter()
            .any(|p| p.as_str() == "/lib.rs"),
        "the new name must be indexed"
    );
    assert!(
        c.search_paths(&ws, "main", 10).unwrap().is_empty(),
        "and the stale term must be gone — an index that accumulates old names returns results \
         for files that no longer exist under them"
    );
}

#[test]
fn a_removed_file_disappears_from_the_index() {
    let (c, ws) = cache();
    c.put_listing(
        &ws,
        &RelPath::root(),
        &[entry("main.rs"), entry("other.rs")],
    )
    .unwrap();
    // A re-listing in which one name vanished.
    c.put_listing(&ws, &RelPath::root(), &[entry("other.rs")])
        .unwrap();
    assert!(
        c.search_paths(&ws, "main", 10).unwrap().is_empty(),
        "a delete must reach the index"
    );
    assert!(
        !c.search_paths(&ws, "other", 10).unwrap().is_empty(),
        "and the survivor stays"
    );
}

#[test]
fn search_finds_by_path_fragment_not_only_by_leading_characters() {
    let (c, ws) = cache();
    let src = RelPath::parse("/src").unwrap();
    c.put_listing(
        &ws,
        &RelPath::root(),
        &[FsEntry {
            name: "src".into(),
            kind: EntryKind::Directory,
            size: 0,
            modified: 0,
        }],
    )
    .unwrap();
    c.put_listing(&ws, &src, &[entry("controller.rs")]).unwrap();

    assert!(
        c.search_paths(&ws, "cont", 10)
            .unwrap()
            .iter()
            .any(|p| p.as_str() == "/src/controller.rs"),
        "a prefix of a path component must match, which is what 'find a file as I type' means"
    );
}

#[test]
fn punctuation_in_a_query_cannot_be_read_as_index_syntax() {
    let (c, ws) = cache();
    c.put_listing(&ws, &RelPath::root(), &[entry("main.rs")])
        .unwrap();
    // These are FTS5 operators. Unquoted they would be a syntax error or a different query.
    for fragment in ["main.", "\"main", "main OR", "src/main", "main*"] {
        let hits = c
            .search_paths(&ws, fragment, 10)
            .unwrap_or_else(|e| panic!("{fragment:?} must not fail the query: {e}"));
        let _ = hits;
    }
}

#[test]
fn an_empty_query_returns_nothing_rather_than_everything() {
    let (c, ws) = cache();
    c.put_listing(&ws, &RelPath::root(), &[entry("main.rs")])
        .unwrap();
    assert!(
        c.search_paths(&ws, "   ", 10).unwrap().is_empty(),
        "an empty fragment must not become a match-all, which would dump the whole workspace \
         into a filter box the moment someone clears it"
    );
}
