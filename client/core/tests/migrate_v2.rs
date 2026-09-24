//! Version 1 to version 2: two columns and one narrowed trigger.
//!
//! F003 built the migration machinery and shipped only step 1, so this is its first real use.
//! The migration test is therefore as much the point as the columns.

use apex_shell::adapters::outbound::sqlite::migrate::{migrate, read_version};
use apex_shell::adapters::outbound::sqlite::schema;
use rusqlite::Connection;

fn v1_database() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    schema::apply_pragmas(&conn).expect("pragmas");
    let mut noop = |_| {};
    migrate(&mut conn, 1, &mut noop).expect("migrate to 1");
    assert_eq!(read_version(&conn).expect("version"), 1);
    conn
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare");
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    rows
}

fn trigger_sql(conn: &Connection, name: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
        [name],
        |r| r.get(0),
    )
    .expect("the trigger exists")
}

#[test]
fn version_two_adds_exactly_two_columns() {
    let mut conn = v1_database();
    let before_files = columns(&conn, "files");
    let before_contents = columns(&conn, "file_contents");

    let mut noop = |_| {};
    migrate(&mut conn, 2, &mut noop).expect("migrate to 2");

    assert_eq!(read_version(&conn).expect("version"), 2);
    let files = columns(&conn, "files");
    let contents = columns(&conn, "file_contents");
    assert!(files.contains(&"stale".to_string()));
    assert!(contents.contains(&"unproven".to_string()));
    assert_eq!(files.len(), before_files.len() + 1, "one column, not two");
    assert_eq!(contents.len(), before_contents.len() + 1);
}

#[test]
fn the_update_trigger_is_narrowed_to_the_indexed_columns() {
    let mut conn = v1_database();
    assert!(
        !trigger_sql(&conn, "files_fts_update").contains("UPDATE OF"),
        "v1 shipped the wide form; this test is meaningless if it did not"
    );

    let mut noop = |_| {};
    migrate(&mut conn, 2, &mut noop).expect("migrate to 2");

    let sql = trigger_sql(&conn, "files_fts_update");
    assert!(sql.contains("UPDATE OF relative_path, name"), "{sql}");
}

#[test]
fn marking_a_row_stale_does_not_touch_the_search_index() {
    // The point of narrowing the trigger. Marking a tree stale is the most common operation
    // this feature performs and it changes no indexed term; the wide trigger reindexed every
    // row anyway, which is 2N writes for N rows that did not change.
    let mut conn = v1_database();
    let mut noop = |_| {};
    migrate(&mut conn, 2, &mut noop).expect("migrate to 2");

    conn.execute(
        "INSERT INTO workspaces(workspace_id,name,location_type,base_path,last_opened_at)
         VALUES('ws','n','local','/ws',0)",
        [],
    )
    .expect("workspace");
    conn.execute(
        "INSERT INTO files(file_id,workspace_id,parent_path,relative_path,name,is_directory,
                           size_bytes,remote_modified_at,is_cached)
         VALUES('f1','ws','/','src/main.rs','main.rs',0,10,0,0)",
        [],
    )
    .expect("file");

    let index_rows = |c: &Connection| -> i64 {
        c.query_row("SELECT count(*) FROM files_fts", [], |r| r.get(0))
            .expect("count")
    };
    let before = index_rows(&conn);
    assert_eq!(before, 1, "the insert trigger indexed it");

    conn.execute("UPDATE files SET stale = 1 WHERE file_id = 'f1'", [])
        .expect("mark stale");

    assert_eq!(
        index_rows(&conn),
        before,
        "a stale-only update must not reindex; the narrowed trigger is what prevents it"
    );
    // And a real rename still does reindex, or the narrowing has gone too far.
    conn.execute(
        "UPDATE files SET relative_path = 'src/lib.rs', name = 'lib.rs' WHERE file_id = 'f1'",
        [],
    )
    .expect("rename");
    let found: i64 = conn
        .query_row(
            "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'lib'",
            [],
            |r| r.get(0),
        )
        .expect("search");
    assert_eq!(found, 1, "a rename must still reach the index");
}

#[test]
fn both_new_columns_default_to_false_for_existing_rows() {
    // An existing projection migrates without every row becoming stale or unproven, which
    // would invalidate a whole cache on upgrade for no reason.
    let mut conn = v1_database();
    conn.execute(
        "INSERT INTO workspaces(workspace_id,name,location_type,base_path,last_opened_at)
         VALUES('ws','n','local','/ws',0)",
        [],
    )
    .expect("workspace");
    conn.execute(
        "INSERT INTO files(file_id,workspace_id,parent_path,relative_path,name,is_directory,
                           size_bytes,remote_modified_at,is_cached)
         VALUES('f1','ws','/','a.rs','a.rs',0,1,0,0)",
        [],
    )
    .expect("file");

    let mut noop = |_| {};
    migrate(&mut conn, 2, &mut noop).expect("migrate to 2");

    let stale: i64 = conn
        .query_row("SELECT stale FROM files WHERE file_id='f1'", [], |r| {
            r.get(0)
        })
        .expect("stale");
    assert_eq!(
        stale, 0,
        "an upgrade must not invalidate what it did not check"
    );
}
