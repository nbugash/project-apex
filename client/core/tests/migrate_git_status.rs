//! Version 2 to version 3: git state re-keyed, and somewhere to keep the branch.
//!
//! **The interesting assertion is not about the new table.** Nothing has ever written a git row,
//! so a migration that dropped the whole database and recreated it would satisfy every check
//! about `git_status` perfectly. What has to survive is everything *else* — the workspaces, the
//! files, the cached content — and that is what a migration can plausibly destroy while looking
//! like it worked.

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

/// A version 2 database with real content in it, so the migration has something to lose.
fn populated_v2() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    schema::apply_pragmas(&conn).expect("pragmas");
    let mut noop = |_| {};
    migrate(&mut conn, 2, &mut noop).expect("migrate to 2");
    conn.execute_batch(
        "INSERT INTO workspaces (workspace_id,name,location_type,base_path,last_opened_at)
             VALUES ('w1','repo','REMOTE','/b',7);
         INSERT INTO files (file_id,workspace_id,parent_path,relative_path,name,
                            is_directory,size_bytes,remote_modified_at,is_cached)
             VALUES ('f1','w1','/','/src/a.rs','a.rs',0,10,0,1);
         INSERT INTO file_contents (file_id,content_blob,sha256_hash)
             VALUES ('f1', X'6162', 'abc');",
    )
    .expect("seed");
    assert_eq!(read_version(&conn).expect("version"), 2);
    conn
}

#[test]
fn the_rest_of_the_projection_survives_the_upgrade() {
    // The assertion that actually constrains the migration. A step that recreated the database
    // would pass every test about git_status below and fail only this one.
    let mut conn = populated_v2();
    let mut noop = |_| {};
    migrate(&mut conn, 3, &mut noop).expect("migrate to 3");

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
        "a git schema change must not touch cached content (§5.3)"
    );
}

#[test]
fn git_state_is_keyed_by_workspace_and_path() {
    // The re-key itself. Keyed on the tree's identity for a file, an untracked path in a folder
    // nobody has expanded could not be stored at all — it has no row to point at.
    let mut conn = populated_v2();
    let mut noop = |_| {};
    migrate(&mut conn, 3, &mut noop).expect("migrate to 3");

    let cols = columns(&conn, "git_status");
    assert!(cols.contains(&"workspace_id".to_string()), "{cols:?}");
    assert!(cols.contains(&"relative_path".to_string()), "{cols:?}");
    assert!(
        !cols.contains(&"file_id".to_string()),
        "git state must not be keyed on the file tree: {cols:?}"
    );
}

#[test]
fn a_path_with_no_file_row_can_carry_state() {
    // The behaviour the re-key exists for, asserted rather than inferred from the columns.
    let mut conn = populated_v2();
    let mut noop = |_| {};
    migrate(&mut conn, 3, &mut noop).expect("migrate to 3");

    conn.execute(
        "INSERT INTO git_status (workspace_id,relative_path,status_type)
             VALUES ('w1','/never/listed.txt','UNTRACKED')",
        [],
    )
    .expect("an untracked path with no tree entry must be storable");

    let kind: String = conn
        .query_row(
            "SELECT status_type FROM git_status WHERE relative_path='/never/listed.txt'",
            [],
            |r| r.get(0),
        )
        .expect("read back");
    assert_eq!(kind, "UNTRACKED");
}

#[test]
fn one_path_holds_one_state() {
    // §4.8 carries one status per path, and the primary key is what makes that true of storage
    // rather than merely of the wire.
    let mut conn = populated_v2();
    let mut noop = |_| {};
    migrate(&mut conn, 3, &mut noop).expect("migrate to 3");
    conn.execute(
        "INSERT INTO git_status (workspace_id,relative_path,status_type) VALUES ('w1','/a','MODIFIED')",
        [],
    )
    .expect("first");
    assert!(
        conn.execute(
            "INSERT INTO git_status (workspace_id,relative_path,status_type) VALUES ('w1','/a','STAGED')",
            [],
        )
        .is_err(),
        "a second state for one path must be refused by the key, not by convention"
    );
}

#[test]
fn forgetting_a_workspace_takes_its_git_state_with_it() {
    // The cascade, which is the reason the foreign key survived the re-key. Without it, closing
    // a workspace would leave its git rows behind to colour the next workspace that reused the
    // identity.
    let mut conn = populated_v2();
    let mut noop = |_| {};
    migrate(&mut conn, 3, &mut noop).expect("migrate to 3");
    conn.execute_batch(
        "INSERT INTO git_status (workspace_id,relative_path,status_type) VALUES ('w1','/a','MODIFIED');
         INSERT INTO git_branch (workspace_id,kind,value) VALUES ('w1','branch','main');
         DELETE FROM workspaces WHERE workspace_id='w1';",
    )
    .expect("cascade");

    let n: i64 = conn
        .query_row("SELECT count(*) FROM git_status", [], |r| r.get(0))
        .expect("count");
    let b: i64 = conn
        .query_row("SELECT count(*) FROM git_branch", [], |r| r.get(0))
        .expect("count");
    assert_eq!((n, b), (0, 0), "git state must not outlive its workspace");
}

#[test]
fn the_branch_has_somewhere_to_live() {
    let mut conn = populated_v2();
    let mut noop = |_| {};
    migrate(&mut conn, 3, &mut noop).expect("migrate to 3");
    let cols = columns(&conn, "git_branch");
    assert!(cols.contains(&"kind".to_string()), "{cols:?}");
    assert!(
        cols.contains(&"value".to_string()),
        "a detached head has a commit but no name, so the value is separate from the kind: {cols:?}"
    );
}
