//! The schema version ladder, and what happens when a step cannot complete.
//!
//! `PRAGMA user_version` rather than a `schema_version` table: SQLite provides it in the file
//! header, it is read without a query, and it cannot itself be the thing that needs migrating.

use super::schema;
use crate::application::ports::workspace_cache::MigrationFailure;
use crate::domain::cache::MaintenancePhase;
use rusqlite::Connection;
use std::path::Path;

pub fn read_version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
        .map(|v| v as u32)
}

/// Apply every step from the current version up to `target`.
///
/// **Each step is one transaction that also sets the new `user_version`.** That is what makes a
/// half-transformed projection unreachable rather than merely unlikely (FR-018b, SC-013b): SQLite's
/// DDL is transactional, so a crash, a kill or a full disk mid-step leaves the file exactly as it
/// was, at the old version, and the next launch retries the same step. There is no moment at which
/// a partial state exists to be observed.
pub fn migrate(
    conn: &mut Connection,
    target: u32,
    progress: &mut dyn FnMut(MaintenancePhase),
) -> Result<(), MigrationFailure> {
    let from = read_version(conn).map_err(|e| MigrationFailure::Step {
        version: 0,
        why: e.to_string(),
    })?;

    if from > target {
        // Written by a newer build. Same treatment as a failed migration: the projection is
        // disposable, and refusing to launch after a downgrade would strand a developer behind a
        // cache the specification itself calls reproducible.
        return Err(MigrationFailure::FromTheFuture {
            found: from,
            expected: target,
        });
    }

    for step in (from + 1)..=target {
        progress(MaintenancePhase::Migrating { from, to: target });
        let tx = conn.transaction().map_err(|e| MigrationFailure::Step {
            version: step,
            why: e.to_string(),
        })?;
        let ddl = match step {
            1 => schema::V1,
            2 => schema::V2,
            other => {
                return Err(MigrationFailure::Step {
                    version: other,
                    why: format!("this build knows no step {other}"),
                })
            }
        };
        tx.execute_batch(ddl).map_err(|e| MigrationFailure::Step {
            version: step,
            why: e.to_string(),
        })?;
        tx.pragma_update(None, "user_version", step as i64)
            .map_err(|e| MigrationFailure::Step {
                version: step,
                why: e.to_string(),
            })?;
        tx.commit().map_err(|e| MigrationFailure::Step {
            version: step,
            why: e.to_string(),
        })?;
    }
    Ok(())
}

/// Delete the projection and everything SQLite keeps beside it.
///
/// **The `-wal` and `-shm` companions are named explicitly.** Deleting only the main database
/// leaves a write-ahead log that SQLite will happily replay into the fresh one, which would carry
/// the very state the rebuild exists to discard.
pub fn discard(path: &Path) -> std::io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let p = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            let mut s = path.as_os_str().to_os_string();
            s.push(suffix);
            std::path::PathBuf::from(s)
        };
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::schema::CURRENT_VERSION;
    use super::*;

    fn noop(_: MaintenancePhase) {}

    #[test]
    fn a_fresh_database_migrates_to_the_current_version() {
        let mut c = Connection::open_in_memory().unwrap();
        schema::apply_pragmas(&c).unwrap();
        assert_eq!(read_version(&c).unwrap(), 0);
        migrate(&mut c, CURRENT_VERSION, &mut noop).expect("migrates");
        assert_eq!(read_version(&c).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn migrating_an_up_to_date_database_does_nothing_and_succeeds() {
        let mut c = Connection::open_in_memory().unwrap();
        schema::apply_pragmas(&c).unwrap();
        migrate(&mut c, CURRENT_VERSION, &mut noop).unwrap();
        migrate(&mut c, CURRENT_VERSION, &mut noop).expect("idempotent");
        assert_eq!(read_version(&c).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn a_database_from_the_future_is_refused_for_rebuilding() {
        let mut c = Connection::open_in_memory().unwrap();
        schema::apply_pragmas(&c).unwrap();
        c.pragma_update(None, "user_version", 99i64).unwrap();
        assert_eq!(
            migrate(&mut c, CURRENT_VERSION, &mut noop),
            Err(MigrationFailure::FromTheFuture {
                found: 99,
                expected: CURRENT_VERSION
            })
        );
    }

    #[test]
    fn progress_is_published_for_each_step() {
        let mut c = Connection::open_in_memory().unwrap();
        schema::apply_pragmas(&c).unwrap();
        let mut seen = Vec::new();
        migrate(&mut c, CURRENT_VERSION, &mut |p| seen.push(p)).unwrap();
        assert!(
            seen.iter()
                .any(|p| matches!(p, MaintenancePhase::Migrating { .. })),
            "a migration must say it is running (FR-018a)"
        );
    }

    #[test]
    fn a_failed_step_leaves_the_version_where_it_was() {
        let mut c = Connection::open_in_memory().unwrap();
        schema::apply_pragmas(&c).unwrap();
        // Occupy a name the DDL needs, so the batch fails partway.
        c.execute_batch("CREATE TABLE files (bogus INTEGER);")
            .unwrap();
        let before = read_version(&c).unwrap();
        let err = migrate(&mut c, CURRENT_VERSION, &mut noop).expect_err("must fail");
        assert!(
            matches!(err, MigrationFailure::Step { version: 1, .. }),
            "{err:?}"
        );
        assert_eq!(
            read_version(&c).unwrap(),
            before,
            "the transaction must have rolled back: a half-transformed projection is the one \
             state that could serve wrong bytes while believing they are right"
        );
        let leftover: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'workspaces'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0, "nothing from the failed step may survive");
    }

    #[test]
    fn discard_removes_the_wal_and_shm_companions_too() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cache.db");
        for suffix in ["", "-wal", "-shm"] {
            std::fs::write(format!("{}{suffix}", db.display()), b"x").unwrap();
        }
        discard(&db).unwrap();
        for suffix in ["", "-wal", "-shm"] {
            assert!(
                !Path::new(&format!("{}{suffix}", db.display())).exists(),
                "a surviving {suffix} file would be replayed into the fresh database, carrying \
                 the state the rebuild exists to discard"
            );
        }
    }
}
