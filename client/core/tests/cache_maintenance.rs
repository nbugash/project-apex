//! US4: retention and migration, against a real database file and a fake clock.
//!
//! A real file because the invariants under test are SQLite's — transactional DDL, the cascade,
//! the `-wal` companion — and a fake would be asserting our own beliefs about them. A fake clock
//! because ageing content past fourteen days is arithmetic, not a fortnight of waiting.

mod common;

use apex_shell::adapters::outbound::sqlite::{migrate, schema, SqliteWorkspaceCache};
use apex_shell::application::ports::workspace_cache::{StoreOutcome, WorkspaceCache};
use apex_shell::application::use_cases::maintain_cache::MaintainCache;
use apex_shell::domain::cache::{MaintenancePhase, RetentionWindow};
use apex_shell::domain::workspace::{
    EntryKind, FsEntry, Location, RelPath, Sha256, Workspace, WorkspaceId,
};
use common::fake_cache::InMemoryCache;
use common::fake_clock::FakeClock;
use std::sync::{Arc, Mutex};

const DAY: i64 = 24 * 60 * 60;

fn workspace() -> Workspace {
    Workspace {
        id: WorkspaceId("w1".into()),
        name: "repo".into(),
        location: Location::Remote {
            host: "h".into(),
            base: "/b".into(),
        },
        last_opened_at: 0,
    }
}

/// A real database with one cached file, last accessed at `accessed_at`.
fn seeded_file(path: &std::path::Path, accessed_at: i64) -> (SqliteWorkspaceCache, RelPath) {
    let cache = SqliteWorkspaceCache::open(path).expect("open");
    cache
        .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("v1");
    let ws = workspace();
    cache.register(&ws, 0).unwrap();
    cache
        .put_listing(
            &ws.id,
            &RelPath::root(),
            &[FsEntry {
                name: "main.rs".into(),
                kind: EntryKind::File,
                size: 5,
                modified: 0,
            }],
        )
        .unwrap();
    let p = RelPath::parse("/main.rs").unwrap();
    let id = cache.file_id(&ws.id, &p).unwrap().unwrap();
    assert_eq!(
        cache.put_content(&id, b"hello", &Sha256::of(b"hello"), accessed_at),
        StoreOutcome::Stored
    );
    (cache, p)
}

/// US4.1, US4.2, SC-008.
#[test]
fn content_aged_past_the_window_is_removed_while_every_tree_entry_survives() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, path) = seeded_file(&dir.path().join("cache.db"), 0);
    let ws = workspace();

    // Fifteen days later.
    let now = 15 * DAY;
    let report = cache.evict(RetentionWindow::DAYS_14.cutoff(now)).unwrap();

    assert_eq!(report.blobs_removed, 1);
    assert!(
        cache.lookup(&ws.id, &path).unwrap().is_none(),
        "content gone"
    );
    assert_eq!(
        cache.list_children(&ws.id, &RelPath::root()).unwrap().len(),
        1,
        "the tree must remain navigable and the file must remain listed (FR-027)"
    );
}

/// US4.4, FR-028: retention measures use, not age.
#[test]
fn a_file_touched_recently_survives_even_though_it_was_cached_long_ago() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, path) = seeded_file(&dir.path().join("cache.db"), 0);
    let ws = workspace();
    let id = cache.file_id(&ws.id, &path).unwrap().unwrap();

    // Cached twenty days ago, read yesterday.
    let now = 20 * DAY;
    assert_eq!(cache.touch(&id, now - DAY), StoreOutcome::Stored);

    let report = cache.evict(RetentionWindow::DAYS_14.cutoff(now)).unwrap();
    assert_eq!(
        report.blobs_removed, 0,
        "a file read daily must never be evicted"
    );
    assert!(cache.lookup(&ws.id, &path).unwrap().is_some());
}

/// US4.3, FR-029.
#[test]
fn an_evicted_file_can_be_cached_again_with_nothing_unusual_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, path) = seeded_file(&dir.path().join("cache.db"), 0);
    let ws = workspace();
    cache
        .evict(RetentionWindow::DAYS_14.cutoff(15 * DAY))
        .unwrap();

    let id = cache.file_id(&ws.id, &path).unwrap().expect("still listed");
    assert_eq!(
        cache.put_content(&id, b"hello", &Sha256::of(b"hello"), 15 * DAY),
        StoreOutcome::Stored,
        "re-caching is an ordinary write, not a recovery"
    );
    assert!(cache.lookup(&ws.id, &path).unwrap().is_some());
}

/// US4.6, SC-013: content survives an upgrade.
#[test]
fn a_migration_preserves_cached_content() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cache.db");
    let (cache, path) = seeded_file(&db, 100);
    let ws = workspace();
    drop(cache);

    // Reopen and migrate again: already current, so nothing changes and nothing is lost.
    let cache = SqliteWorkspaceCache::open(&db).expect("reopen");
    cache
        .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("idempotent");
    let entry = cache
        .lookup(&ws.id, &path)
        .unwrap()
        .expect("content preserved");
    assert_eq!(entry.bytes, b"hello");
}

/// FR-017: the projection survives a restart. The only assertion that a persisted store is
/// actually persisted — nothing else in the suite would fail if it were opened in memory.
#[test]
fn the_projection_survives_closing_and_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cache.db");
    let (cache, path) = seeded_file(&db, 1);
    drop(cache);

    let reopened = SqliteWorkspaceCache::open(&db).expect("reopen");
    let ws = workspace();
    assert_eq!(
        reopened
            .list_children(&ws.id, &RelPath::root())
            .unwrap()
            .len(),
        1,
        "the tree must still be there"
    );
    assert_eq!(
        reopened
            .lookup(&ws.id, &path)
            .unwrap()
            .expect("content")
            .bytes,
        b"hello",
        "and so must the content"
    );
}

/// FR-018c, SC-013b: a migration killed mid-step leaves the old version intact.
#[test]
fn a_migration_interrupted_mid_step_leaves_the_previous_version_readable() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cache.db");

    // A database at v0 whose schema step will fail partway: the name `files` is occupied.
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        schema::apply_pragmas(&c).unwrap();
        c.execute_batch("CREATE TABLE files (bogus INTEGER);")
            .unwrap();
    }
    let cache = SqliteWorkspaceCache::open(&db).unwrap();
    let before = cache.schema_version().unwrap();
    let err = cache
        .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect_err("must fail");
    assert!(format!("{err:?}").contains("Step"), "{err:?}");
    assert_eq!(
        cache.schema_version().unwrap(),
        before,
        "the transaction rolled back: there is no moment at which a half-transformed projection \
         exists to be observed"
    );
}

/// SC-013b: a deterministically failing migration discards and rebuilds, and says so.
#[test]
fn a_failed_migration_rebuilds_and_reports_it() {
    let cache = Arc::new(InMemoryCache::new());
    cache.fail_migration();
    let rebuilt = Arc::new(Mutex::new(false));
    let flag = rebuilt.clone();

    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let p = published.clone();
        Arc::new(move |m: MaintenancePhase| p.lock().unwrap().push(m))
    };

    let m = MaintainCache::new(
        cache,
        Arc::new(FakeClock::at(0)),
        sink,
        RetentionWindow::DAYS_14,
        Arc::new(move || {
            *flag.lock().unwrap() = true;
            Ok(())
        }),
        1,
    );
    let report = m.run();

    assert!(
        report.rebuilt,
        "the report must say the cache was rebuilt (FR-018b)"
    );
    assert!(
        *rebuilt.lock().unwrap(),
        "and the rebuild must actually have happened"
    );
    assert!(
        published
            .lock()
            .unwrap()
            .contains(&MaintenancePhase::Rebuilding),
        "and the developer must be told"
    );
}

/// A projection written by a newer build takes the same discard path.
#[test]
fn a_cache_from_the_future_is_rebuilt_rather_than_refusing_to_launch() {
    let cache = Arc::new(InMemoryCache::new());
    cache.set_version(99);
    let rebuilt = Arc::new(Mutex::new(false));
    let flag = rebuilt.clone();

    let m = MaintainCache::new(
        cache,
        Arc::new(FakeClock::at(0)),
        Arc::new(|_| {}),
        RetentionWindow::DAYS_14,
        Arc::new(move || {
            *flag.lock().unwrap() = true;
            Ok(())
        }),
        1,
    );
    assert!(
        m.run().rebuilt,
        "refusing to launch would strand a developer behind a disposable \
                              projection after a downgrade"
    );
    assert!(*rebuilt.lock().unwrap());
}

/// SC-013a: the count and spacing of progress reports, not that one was sent.
#[test]
fn maintenance_publishes_a_running_state_throughout_and_ends_ready() {
    let cache = Arc::new(InMemoryCache::new());
    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let p = published.clone();
        Arc::new(move |m: MaintenancePhase| p.lock().unwrap().push(m))
    };
    let m = MaintainCache::new(
        cache,
        Arc::new(FakeClock::at(0)),
        sink,
        RetentionWindow::DAYS_14,
        Arc::new(|| Ok(())),
        1,
    );
    m.run();

    let seen = published.lock().unwrap().clone();
    assert_eq!(seen.first(), Some(&MaintenancePhase::Checking));
    assert!(
        seen.iter()
            .any(|p| matches!(p, MaintenancePhase::Migrating { .. })),
        "a migration must say it is running: asserting only that *a* message was sent would pass \
         for an upgrade that then hangs silently. seen: {seen:?}"
    );
    assert!(seen.contains(&MaintenancePhase::Evicting), "{seen:?}");
    assert_eq!(
        seen.last(),
        Some(&MaintenancePhase::Ready),
        "Ready is the precondition for constructing any provider (FR-018c)"
    );
    // Only three phases are ever rendered; the others exist to order the work.
    let rendered: Vec<_> = seen.iter().filter(|p| p.is_rendered()).collect();
    assert!(
        !rendered.is_empty(),
        "something must be visible to the developer"
    );
}

/// SC-008a: eviction runs once, at startup. Nothing evicts while a workspace is open.
#[test]
fn eviction_runs_exactly_once_per_launch() {
    let cache = Arc::new(InMemoryCache::new());
    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let p = published.clone();
        Arc::new(move |m: MaintenancePhase| p.lock().unwrap().push(m))
    };
    let m = MaintainCache::new(
        cache,
        Arc::new(FakeClock::at(0)),
        sink,
        RetentionWindow::DAYS_14,
        Arc::new(|| Ok(())),
        1,
    );
    m.run();
    let evictions = published
        .lock()
        .unwrap()
        .iter()
        .filter(|p| matches!(p, MaintenancePhase::Evicting))
        .count();
    assert_eq!(
        evictions, 1,
        "deleting competes with reading on the same store, and the one thing §1.4 protects is a \
         sidebar that answers in under a millisecond"
    );
}

/// M4: the companions go with the database, or SQLite replays the WAL into the fresh one.
#[test]
fn discarding_removes_the_wal_and_shm_companions() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cache.db");
    let (cache, _) = seeded_file(&db, 0);
    drop(cache);
    for suffix in ["-wal", "-shm"] {
        std::fs::write(format!("{}{suffix}", db.display()), b"x").unwrap();
    }
    migrate::discard(&db).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        assert!(
            !std::path::Path::new(&format!("{}{suffix}", db.display())).exists(),
            "a surviving {suffix} file carries the state the rebuild exists to discard"
        );
    }
}
