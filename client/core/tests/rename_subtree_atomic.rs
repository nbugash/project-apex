//! A subtree rename is one transaction (research.md, *Directory rename with a subtree*).
//!
//! A partially renamed subtree is a projection that disagrees with itself: some rows under the
//! old prefix, some under the new, and no way to tell which listing is true. That is worse than
//! a stale projection, which is at least consistently out of date.

use apex_shell::adapters::outbound::sqlite::migrate::read_version;
use apex_shell::adapters::outbound::sqlite::schema;
use rusqlite::Connection;

/// The property, asserted against SQLite itself rather than through the cache: a statement that
/// fails part-way leaves the rows it had already written untouched.
#[test]
fn a_statement_that_fails_part_way_rolls_back_entirely() {
    let mut conn = Connection::open_in_memory().expect("open");
    schema::apply_pragmas(&conn).expect("pragmas");
    let mut noop = |_| {};
    apex_shell::adapters::outbound::sqlite::migrate::migrate(
        &mut conn,
        schema::CURRENT_VERSION,
        &mut noop,
    )
    .expect("migrate");
    assert_eq!(
        read_version(&conn).expect("version"),
        schema::CURRENT_VERSION
    );

    conn.execute(
        "INSERT INTO workspaces(workspace_id,name,location_type,base_path,last_opened_at)
         VALUES('ws','n','local','/ws',0)",
        [],
    )
    .expect("workspace");
    for (id, path) in [("f1", "/src/a.rs"), ("f2", "/src/b.rs")] {
        conn.execute(
            "INSERT INTO files(file_id,workspace_id,parent_path,relative_path,name,is_directory,
                               size_bytes,remote_modified_at,is_cached)
             VALUES(?1,'ws','/src',?2,'x',0,1,0,0)",
            rusqlite::params![id, path],
        )
        .expect("file");
    }

    let tx = conn.transaction().expect("transaction");
    tx.execute(
        "UPDATE files SET relative_path='/syntax/a.rs' WHERE file_id='f1'",
        [],
    )
    .expect("first row rewritten");
    // The second collides with the first: `UNIQUE(workspace_id, relative_path)` is checked per
    // row, so a destination that already exists aborts the statement rather than merging.
    let collision = tx.execute(
        "UPDATE files SET relative_path='/syntax/a.rs' WHERE file_id='f2'",
        [],
    );
    assert!(
        collision.is_err(),
        "a duplicate destination must not be accepted"
    );
    drop(tx); // rolled back, because it was never committed

    let surviving: String = conn
        .query_row(
            "SELECT relative_path FROM files WHERE file_id='f1'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(
        surviving, "/src/a.rs",
        "the first rewrite was rolled back with the rest; a half-renamed subtree is unreachable"
    );
}
