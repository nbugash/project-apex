//! Schema version 1: the whole of §5.2, plus the synchronisation triggers this feature added.
//!
//! The **whole** schema, including `git_status`, which nothing in F003 writes. That is not
//! speculative generality: §5.2 is canonical under A-B5, so these tables are specified rather than
//! anticipated. Creating `git_status` now costs one statement; creating it in F011 instead costs a
//! migration, a migration test, a second schema version, and a migration run on every installation
//! in existence — to add a table that was already written down.

use rusqlite::Connection;

/// What shape this build reads and writes.
pub const CURRENT_VERSION: u32 = 2;

/// Pragmas that must be set on **every** connection, not only at creation.
///
/// `foreign_keys` is per-connection in SQLite and defaults **off**. A connection that forgets it
/// gets cascade-free deletes, so forgetting a workspace would leave its files and blobs behind —
/// the projection would leak rows and nothing would say so. `journal_mode` is persistent, but
/// setting it is idempotent and cheap, and stating all three together is what makes the
/// requirement legible.
pub fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

/// The version 1 DDL, exactly as §5.2 specifies it.
pub const V1: &str = r#"
CREATE TABLE workspaces (
    workspace_id    TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    location_type   TEXT NOT NULL,
    base_path       TEXT NOT NULL,
    ssh_host        TEXT,
    last_opened_at  INTEGER NOT NULL
);

CREATE TABLE files (
    file_id            TEXT PRIMARY KEY,
    workspace_id       TEXT NOT NULL,
    parent_path        TEXT NOT NULL,
    relative_path      TEXT NOT NULL,
    name               TEXT NOT NULL,
    is_directory       INTEGER NOT NULL,
    size_bytes         INTEGER NOT NULL,
    remote_modified_at INTEGER NOT NULL,
    last_cached_at     INTEGER,
    last_accessed_at   INTEGER,
    is_cached          INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id) ON DELETE CASCADE,
    UNIQUE(workspace_id, relative_path)
);

CREATE INDEX idx_files_parent ON files(workspace_id, parent_path);
CREATE INDEX idx_files_lookup ON files(workspace_id, relative_path);

CREATE TABLE file_contents (
    file_id      TEXT PRIMARY KEY,
    content_blob BLOB,
    sha256_hash  TEXT NOT NULL,
    FOREIGN KEY(file_id) REFERENCES files(file_id) ON DELETE CASCADE
);

CREATE TABLE git_status (
    file_id       TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    status_type   TEXT NOT NULL,
    FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id) ON DELETE CASCADE
);

CREATE INDEX idx_git_status_lookup ON git_status(workspace_id, relative_path);

CREATE VIRTUAL TABLE files_fts USING fts5(
    relative_path,
    name,
    content='files',
    content_rowid='rowid'
);

CREATE TRIGGER files_fts_insert AFTER INSERT ON files BEGIN
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;

CREATE TRIGGER files_fts_delete AFTER DELETE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
END;

CREATE TRIGGER files_fts_update AFTER UPDATE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;
"#;

/// Version 2: what F004 needs, and nothing else.
///
/// Only the delta. `migrate` runs each step in sequence, so this executes against a database
/// on which V1 has already run -- repeating V1 here would fail on the first `CREATE TABLE`.
///
/// The trigger is the part that would have been missed. V1 shipped `files_fts_update` as
/// `AFTER UPDATE ON files`, with no `OF` clause, so **every** update fires a delete and
/// reinsert against the FTS5 index -- including one that touches only `stale` and changes no
/// indexed term. Marking a tree stale is the most common operation this feature performs and
/// it changes no term at all, so without the narrowing it would cost 2N pointless index writes
/// for N rows. §5.2 was amended; because V1 already exists in the field, the migration drops
/// and recreates rather than simply defining it. The body is unchanged.
///
/// Adding the two columns disturbs none of the three triggers on its own: `ADD COLUMN` fires
/// no trigger, and each trigger names `rowid`, `relative_path` and `name` explicitly rather
/// than using `*`, so the external-content rowids are untouched.
pub const V2: &str = r#"
ALTER TABLE files         ADD COLUMN stale    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE file_contents ADD COLUMN unproven INTEGER NOT NULL DEFAULT 0;

DROP TRIGGER files_fts_update;
CREATE TRIGGER files_fts_update AFTER UPDATE OF relative_path, name ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let c = Connection::open_in_memory().expect("in-memory db");
        apply_pragmas(&c).expect("pragmas");
        c.execute_batch(V1).expect("v1 schema");
        c
    }

    #[test]
    fn the_schema_creates_every_table_and_trigger_5_2_specifies() {
        let c = fresh();
        let names: Vec<String> = c
            .prepare(
                "SELECT name FROM sqlite_master WHERE type IN ('table','trigger') ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for expected in [
            "file_contents",
            "files",
            "files_fts",
            "files_fts_delete",
            "files_fts_insert",
            "files_fts_update",
            "git_status",
            "workspaces",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn foreign_keys_are_on_and_the_cascade_actually_fires() {
        let c = fresh();
        c.execute_batch(
            "INSERT INTO workspaces (workspace_id,name,location_type,base_path,last_opened_at)
                VALUES ('w1','n','REMOTE','/b',0);
             INSERT INTO files (file_id,workspace_id,parent_path,relative_path,name,
                                is_directory,size_bytes,remote_modified_at,is_cached)
                VALUES ('f1','w1','/','/a.rs','a.rs',0,1,0,1);
             INSERT INTO file_contents (file_id,content_blob,sha256_hash)
                VALUES ('f1', x'00', 'deadbeef');",
        )
        .unwrap();
        c.execute("DELETE FROM workspaces WHERE workspace_id = 'w1'", [])
            .unwrap();
        let files: i64 = c
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        let blobs: i64 = c
            .query_row("SELECT count(*) FROM file_contents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            (files, blobs),
            (0, 0),
            "the cascade depends on the per-connection foreign_keys pragma; without it this \
             silently leaves orphans and nothing says so"
        );
    }

    /// The defect §5.2 shipped with: an external-content FTS5 table SQLite does not maintain.
    #[test]
    fn the_fts_index_tracks_inserts_updates_and_deletes() {
        let c = fresh();
        c.execute_batch(
            "INSERT INTO workspaces (workspace_id,name,location_type,base_path,last_opened_at)
                VALUES ('w1','n','REMOTE','/b',0);
             INSERT INTO files (file_id,workspace_id,parent_path,relative_path,name,
                                is_directory,size_bytes,remote_modified_at,is_cached)
                VALUES ('f1','w1','/src','/src/main.rs','main.rs',0,1,0,0);",
        )
        .unwrap();
        let hits = |c: &Connection, q: &str| -> i64 {
            c.query_row(
                "SELECT count(*) FROM files_fts WHERE files_fts MATCH ?1",
                [q],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(hits(&c, "main"), 1, "insert must reach the index");

        c.execute(
            "UPDATE files SET relative_path='/src/lib.rs', name='lib.rs' WHERE file_id='f1'",
            [],
        )
        .unwrap();
        assert_eq!(hits(&c, "main"), 0, "the stale term must be gone");
        assert_eq!(hits(&c, "lib"), 1, "the new term must be present");

        c.execute("DELETE FROM files WHERE file_id='f1'", [])
            .unwrap();
        assert_eq!(hits(&c, "lib"), 0, "delete must reach the index");
    }

    /// Without the triggers the table is created empty and stays empty — and returns nothing
    /// quickly, with no error, which is the worst available failure. This proves the assertion
    /// above would actually catch their absence.
    #[test]
    fn without_the_triggers_the_index_would_silently_return_nothing() {
        let c = Connection::open_in_memory().unwrap();
        apply_pragmas(&c).unwrap();
        // The §5.2 schema with the trigger statements removed.
        let without: String = V1
            .split("CREATE TRIGGER")
            .next()
            .expect("the DDL before the first trigger")
            .to_string();
        c.execute_batch(&without).unwrap();
        c.execute_batch(
            "INSERT INTO workspaces (workspace_id,name,location_type,base_path,last_opened_at)
                VALUES ('w1','n','REMOTE','/b',0);
             INSERT INTO files (file_id,workspace_id,parent_path,relative_path,name,
                                is_directory,size_bytes,remote_modified_at,is_cached)
                VALUES ('f1','w1','/src','/src/main.rs','main.rs',0,1,0,0);",
        )
        .unwrap();
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'main'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 0,
            "this is the failure mode the triggers prevent: no rows, no error, no clue"
        );
    }
}
