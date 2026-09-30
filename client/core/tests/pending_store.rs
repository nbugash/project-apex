//! Work saved while offline survives the client being closed.
//!
//! **Asserted on a reopened store, not a live one.** An in-memory map beside the database
//! satisfies every assertion that keeps one handle open, and "the edit is still there after a
//! relaunch" is the entire promise of offline editing (SC-002). Only closing the store and opening
//! it again distinguishes persisted from remembered.

use apex_shell::adapters::outbound::sqlite::schema::CURRENT_VERSION;
use apex_shell::adapters::outbound::sqlite::SqliteWorkspaceCache;
use apex_shell::application::ports::workspace_cache::{PendingEdit, WorkspaceCache};
use apex_shell::domain::workspace::{Location, RelPath, Sha256, Workspace, WorkspaceId};

fn workspace(id: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(id.into()),
        name: "repo".into(),
        location: Location::Local {
            base: "/repo".into(),
        },
        last_opened_at: 0,
    }
}

/// A store on disk, at the current schema, with the named workspaces registered.
fn opened(path: &std::path::Path, ids: &[&str]) -> SqliteWorkspaceCache {
    let store = SqliteWorkspaceCache::open(path).expect("open");
    store
        .migrate_to(CURRENT_VERSION, &mut |_| {})
        .expect("migrate");
    for id in ids {
        store.register(&workspace(id), 0).expect("register");
    }
    store
}

fn edit(content: &str, base: Option<&str>) -> PendingEdit {
    PendingEdit {
        content: content.as_bytes().to_vec(),
        base: base.map(|b| (b.as_bytes().to_vec(), Sha256::of(b.as_bytes()))),
        mergeable: true,
        retained_at: 42,
    }
}

fn path(p: &str) -> RelPath {
    RelPath::parse(p).expect("a valid path")
}

#[test]
fn an_offline_edit_written_by_one_session_is_there_for_the_next() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    {
        let store = opened(&db, &["w1"]);
        store
            .retain_edit(
                &WorkspaceId("w1".into()),
                &path("/src/a.rs"),
                &edit("local", Some("base")),
            )
            .expect("retain");
    }

    // A new handle on the same file. Everything above is gone from memory.
    let store = SqliteWorkspaceCache::open(&db).expect("reopen");
    let pending = store
        .pending_edits(&WorkspaceId("w1".into()))
        .expect("read back");
    assert_eq!(pending.len(), 1, "the edit must survive a relaunch");
    let (p, e) = &pending[0];
    assert_eq!(p.as_str(), "/src/a.rs");
    assert_eq!(e.content, b"local");
    let (base, hash) = e.base.as_ref().expect("the base travels with the edit");
    assert_eq!(base, b"base");
    assert_eq!(
        hash,
        &Sha256::of(b"base"),
        "the stored digest must be of the decompressed base"
    );
}

#[test]
fn a_path_with_no_file_row_and_no_cached_content_can_carry_work() {
    // A file created offline. It has no `files` row and no `file_contents` row, which is why the
    // table is keyed by path: there is no tree identity to key on.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = opened(&db, &["w1"]);
    store
        .retain_edit(
            &WorkspaceId("w1".into()),
            &path("/created/offline.rs"),
            &edit("new", None),
        )
        .expect("retain");

    let pending = store
        .pending_edits(&WorkspaceId("w1".into()))
        .expect("read");
    assert_eq!(pending.len(), 1);
    assert!(
        pending[0].1.base.is_none(),
        "a file created offline has nothing to differ from"
    );
}

#[test]
fn forgetting_removes_exactly_one_workspace_and_one_path() {
    // Three rows across two workspaces. A DELETE that forgot either half of the key would take
    // something it was not asked to, and the row it takes is a developer's unreconciled work.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = opened(&db, &["w1", "w2"]);
    let (w1, w2) = (WorkspaceId("w1".into()), WorkspaceId("w2".into()));
    store
        .retain_edit(&w1, &path("/a.rs"), &edit("a", Some("b")))
        .expect("retain");
    store
        .retain_edit(&w1, &path("/b.rs"), &edit("a", Some("b")))
        .expect("retain");
    store
        .retain_edit(&w2, &path("/a.rs"), &edit("a", Some("b")))
        .expect("retain");

    store.forget_pending(&w1, &path("/a.rs")).expect("forget");

    let left_w1: Vec<String> = store
        .pending_edits(&w1)
        .expect("read")
        .into_iter()
        .map(|(p, _)| p.as_str().to_string())
        .collect();
    assert_eq!(
        left_w1,
        vec!["/b.rs".to_string()],
        "only the named path goes"
    );
    assert_eq!(
        store.pending_edits(&w2).expect("read").len(),
        1,
        "the other workspace's identically-named path must be untouched"
    );
}

#[test]
fn pending_edits_is_scoped_to_one_workspace() {
    // The table is keyed `(workspace_id, relative_path)` so this can be scoped. A query that
    // forgot the first half of the key would still return plausible rows -- somebody else's.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = opened(&db, &["w1", "w2"]);
    store
        .retain_edit(
            &WorkspaceId("w1".into()),
            &path("/mine.rs"),
            &edit("mine", Some("b")),
        )
        .expect("retain");
    store
        .retain_edit(
            &WorkspaceId("w2".into()),
            &path("/theirs.rs"),
            &edit("theirs", Some("b")),
        )
        .expect("retain");

    let mine = store
        .pending_edits(&WorkspaceId("w1".into()))
        .expect("read");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].0.as_str(), "/mine.rs");
}

#[test]
fn a_second_offline_save_replaces_the_content_and_keeps_the_base() {
    // FR-011b, and the most dangerous property in the feature. Re-deriving the base from the new
    // local content would make the eventual merge compare local against local and return a clean
    // merge that is wrong -- no assertion about success would notice, and the developer is handed a
    // file nobody wrote.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = opened(&db, &["w1"]);
    let w1 = WorkspaceId("w1".into());
    store
        .retain_edit(&w1, &path("/a.rs"), &edit("first", Some("host")))
        .expect("first save");
    store
        .retain_edit(&w1, &path("/a.rs"), &edit("second", Some("first")))
        .expect("second save");
    store
        .retain_edit(&w1, &path("/a.rs"), &edit("third", Some("second")))
        .expect("third save");

    let pending = store.pending_edits(&w1).expect("read");
    assert_eq!(
        pending.len(),
        1,
        "one row per path, replaced not accumulated"
    );
    let e = &pending[0].1;
    assert_eq!(e.content, b"third", "the latest save is what reconciles");
    let (base, _) = e.base.as_ref().expect("a base");
    assert_eq!(
        base, b"host",
        "the base must still be the content the host confirmed, not the previous local save"
    );
}

#[test]
fn the_row_holds_content_and_a_base_and_nothing_else() {
    // FR-034's bound: what is retained is a file's content and the base it came from. There is no
    // outbox for arbitrary operations, and the schema is where that is enforced rather than
    // promised -- a column for a queued command is how an outbox arrives one field at a time.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let _store = opened(&db, &["w1"]);
    let conn = rusqlite::Connection::open(&db).expect("open directly");
    let mut stmt = conn
        .prepare("PRAGMA table_info(pending_edits)")
        .expect("prepare");
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");

    let mut expected = vec![
        "workspace_id",
        "relative_path",
        "content_blob",
        "base_blob",
        "base_sha256",
        "mergeable",
        "retained_at",
    ];
    expected.sort_unstable();
    let mut got: Vec<&str> = cols.iter().map(String::as_str).collect();
    got.sort_unstable();
    assert_eq!(
        got, expected,
        "a column beyond these is how an outbox for arbitrary operations arrives (FR-034)"
    );
}

#[test]
fn a_row_whose_path_does_not_validate_is_dropped_rather_than_repaired() {
    // Principle VI: re-validated on read as well as on write. Written through SQL rather than
    // through the port, because the port is what is supposed to make this state impossible -- a
    // test that can only build the row the legal way asserts nothing about the illegal one.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = opened(&db, &["w1"]);
    let w1 = WorkspaceId("w1".into());
    store
        .retain_edit(&w1, &path("/good.rs"), &edit("ok", Some("b")))
        .expect("retain");
    {
        let conn = rusqlite::Connection::open(&db).expect("open directly");
        conn.execute(
            "INSERT INTO pending_edits
                 (workspace_id,relative_path,content_blob,base_blob,base_sha256,mergeable,retained_at)
                 VALUES ('w1','/../escape.rs', X'6162', NULL, NULL, 1, 1)",
            [],
        )
        .expect("insert a path the port would refuse");
    }

    let pending = store.pending_edits(&w1).expect("read");
    let paths: Vec<&str> = pending.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        vec!["/good.rs"],
        "a traversal path must be dropped, not normalised into a destination for someone's work"
    );
}

#[test]
fn one_base_column_without_the_other_is_treated_as_unmergeable() {
    // data-model.md's validation rule and design.md's error table. Merging content against a base
    // that is absent rather than empty is mutation 10's wrong-clean-merge shape arriving from a
    // malformed row, so the file must prompt instead.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = opened(&db, &["w1"]);
    {
        let conn = rusqlite::Connection::open(&db).expect("open directly");
        conn.execute(
            "INSERT INTO pending_edits
                 (workspace_id,relative_path,content_blob,base_blob,base_sha256,mergeable,retained_at)
                 VALUES ('w1','/half.rs', X'6162', X'61', NULL, 1, 1)",
            [],
        )
        .expect("insert a half-based row");
    }

    let pending = store
        .pending_edits(&WorkspaceId("w1".into()))
        .expect("read");
    assert_eq!(
        pending.len(),
        1,
        "the row is kept -- it is work, not rubbish"
    );
    let e = &pending[0].1;
    assert!(e.base.is_none(), "half a base is no base");
    assert!(
        !e.mergeable,
        "a file with half a base must prompt rather than merge against nothing"
    );
}
