//! Version 3 to version 4: somewhere to keep work the host has not seen.
//!
//! **The interesting assertion is not about the new table.** Nothing has ever written a
//! `pending_edits` row at migration time, so a step that dropped the database and recreated it
//! would satisfy every check about the new table perfectly. What has to survive is everything
//! *else* — the workspaces, the files, the cached content, and the git state F011 added — and that
//! is what a migration can plausibly destroy while looking like it worked. F011's V3 test learned
//! to make this assertion; this one inherits it, one version on and with more to lose.

use apex_shell::adapters::outbound::sqlite::migrate::{migrate, read_version};
use apex_shell::adapters::outbound::sqlite::schema;
use rusqlite::Connection;

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare");
    stmt.query_map([], |r| r.get::<_, String>(1))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// A version 3 database with real content in it, so the migration has something to lose.
///
/// Seeded through every table V3 leaves behind, git state included. A migration that recreated the
/// database would pass every assertion about `pending_edits` and fail on each of these.
fn populated_v3() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    schema::apply_pragmas(&conn).expect("pragmas");
    let mut noop = |_| {};
    migrate(&mut conn, 3, &mut noop).expect("migrate to 3");
    conn.execute_batch(
        "INSERT INTO workspaces (workspace_id,name,location_type,base_path,last_opened_at)
             VALUES ('w1','repo','REMOTE','/b',7);
         INSERT INTO files (file_id,workspace_id,parent_path,relative_path,name,
                            is_directory,size_bytes,remote_modified_at,is_cached)
             VALUES ('f1','w1','/','/src/a.rs','a.rs',0,10,0,1);
         INSERT INTO file_contents (file_id,content_blob,sha256_hash)
             VALUES ('f1', X'6162', 'abc');
         INSERT INTO git_status (workspace_id,relative_path,status_type)
             VALUES ('w1','/src/a.rs','MODIFIED');
         INSERT INTO git_branch (workspace_id,kind,value)
             VALUES ('w1','branch','main');",
    )
    .expect("seed");
    assert_eq!(read_version(&conn).expect("version"), 3);
    conn
}

#[test]
fn the_rest_of_the_projection_survives_the_upgrade() {
    // The assertion that actually constrains the migration. A step that recreated the database
    // would pass every test about pending_edits below and fail only this one.
    let mut conn = populated_v3();
    let mut noop = |_| {};
    migrate(&mut conn, 4, &mut noop).expect("migrate to 4");

    let ws: String = conn
        .query_row(
            "SELECT name FROM workspaces WHERE workspace_id='w1'",
            [],
            |r| r.get(0),
        )
        .expect("the workspace must survive");
    assert_eq!(ws, "repo");

    let path: String = conn
        .query_row(
            "SELECT relative_path FROM files WHERE file_id='f1'",
            [],
            |r| r.get(0),
        )
        .expect("the file must survive");
    assert_eq!(path, "/src/a.rs");

    let hash: String = conn
        .query_row(
            "SELECT sha256_hash FROM file_contents WHERE file_id='f1'",
            [],
            |r| r.get(0),
        )
        .expect("the cached content must survive");
    assert_eq!(
        hash, "abc",
        "adding a table must not touch cached content (§5.3)"
    );

    let status: String = conn
        .query_row(
            "SELECT status_type FROM git_status WHERE relative_path='/src/a.rs'",
            [],
            |r| r.get(0),
        )
        .expect("F011's git state must survive");
    assert_eq!(status, "MODIFIED");

    let branch: String = conn
        .query_row(
            "SELECT value FROM git_branch WHERE workspace_id='w1'",
            [],
            |r| r.get(0),
        )
        .expect("F011's branch must survive");
    assert_eq!(branch, "main");
}

#[test]
fn the_upgrade_is_additive() {
    // V3 dropped and recreated a table. This one must not: anything it drops is data a developer
    // has, because by version 4 every table in the projection has been written to in the field.
    let mut conn = populated_v3();
    let before: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    let mut noop = |_| {};
    migrate(&mut conn, 4, &mut noop).expect("migrate to 4");
    let after: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };

    let lost: Vec<&String> = before.iter().filter(|t| !after.contains(t)).collect();
    assert!(lost.is_empty(), "version 4 must add only: lost {lost:?}");
    let gained: Vec<&String> = after.iter().filter(|t| !before.contains(t)).collect();
    assert_eq!(
        gained,
        vec![&"pending_edits".to_string()],
        "exactly one table is added"
    );
}

#[test]
fn pending_work_is_keyed_by_workspace_and_path() {
    // Keyed on the tree's identity for a file, a file created offline could not be stored at all:
    // it has no `files` row to point at. V3 is the record of what keying on the tree costs.
    let mut conn = populated_v3();
    let mut noop = |_| {};
    migrate(&mut conn, 4, &mut noop).expect("migrate to 4");

    let cols = columns(&conn, "pending_edits");
    assert!(cols.contains(&"workspace_id".to_string()), "{cols:?}");
    assert!(cols.contains(&"relative_path".to_string()), "{cols:?}");
    assert!(
        !cols.contains(&"file_id".to_string()),
        "pending work must not be keyed on the file tree: {cols:?}"
    );
    assert!(
        cols.contains(&"base_blob".to_string()),
        "the base is stored as content, not referenced: {cols:?}"
    );
}

#[test]
fn a_path_with_no_file_row_can_carry_work() {
    // The behaviour the key exists for, asserted rather than inferred from the columns. This is a
    // file created offline: no tree row, no cached content, and work that must not be lost.
    let mut conn = populated_v3();
    let mut noop = |_| {};
    migrate(&mut conn, 4, &mut noop).expect("migrate to 4");

    conn.execute(
        "INSERT INTO pending_edits
             (workspace_id,relative_path,content_blob,base_blob,base_sha256,mergeable,retained_at)
             VALUES ('w1','/created/offline.rs', X'6e6577', NULL, NULL, 1, 42)",
        [],
    )
    .expect("a path with no tree entry and no cached content must be storable");

    let (blob, base): (Vec<u8>, Option<Vec<u8>>) = conn
        .query_row(
            "SELECT content_blob, base_blob FROM pending_edits
                 WHERE relative_path='/created/offline.rs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read back");
    assert_eq!(blob, b"new");
    assert!(
        base.is_none(),
        "a file created offline has nothing to differ from"
    );
}

#[test]
fn forgetting_a_workspace_takes_its_pending_work_with_it() {
    // The foreign key, asserted rather than read off the DDL. `foreign_keys` is per-connection in
    // SQLite and defaults off, so a cascade is a property of the connection as much as the schema.
    let mut conn = populated_v3();
    let mut noop = |_| {};
    migrate(&mut conn, 4, &mut noop).expect("migrate to 4");
    conn.execute(
        "INSERT INTO pending_edits
             (workspace_id,relative_path,content_blob,base_blob,base_sha256,mergeable,retained_at)
             VALUES ('w1','/src/a.rs', X'6162', X'61', 'abc', 1, 42)",
        [],
    )
    .expect("insert");

    conn.execute("DELETE FROM workspaces WHERE workspace_id='w1'", [])
        .expect("forget the workspace");

    let left: i64 = conn
        .query_row("SELECT COUNT(*) FROM pending_edits", [], |r| r.get(0))
        .expect("count");
    assert_eq!(
        left, 0,
        "forgetting a workspace must not leave its pending work behind"
    );
}
