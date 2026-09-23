//! Wiring the workspace layer, with the ordering FR-018c and FR-026a require.
//!
//! Kept in its own file because the ordering **is** the requirement. A projection must never be
//! read with a schema it was not written for, at any point, and eviction must never run while a
//! workspace is open. Both are satisfied by maintenance completing before any provider exists —
//! and that is enforced here by a type, not by a comment asking the next person to remember.

use crate::adapters::outbound::sqlite::{migrate, schema, SqliteWorkspaceCache};
use crate::adapters::outbound::system_clock::SystemClock;
use crate::application::ports::clock::Clock;
use crate::application::ports::workspace_cache::WorkspaceCache;
use crate::application::use_cases::maintain_cache::{
    MaintainCache, MaintenanceReport, PhasePublisher,
};
use crate::domain::cache::RetentionWindow;
use std::path::PathBuf;
use std::sync::Arc;

/// A cache that has been through maintenance.
///
/// The only way to obtain one is `prepare_cache`, which runs maintenance first. Nothing that
/// consumes a cache accepts anything else, so "maintenance ran before any provider was built" is
/// checked by the compiler rather than asserted in a comment.
pub struct ReadyCache {
    cache: Arc<dyn WorkspaceCache>,
    pub report: MaintenanceReport,
}

impl ReadyCache {
    /// Hand out the cache. Available only from a `ReadyCache`, which is the point.
    pub fn get(&self) -> Arc<dyn WorkspaceCache> {
        self.cache.clone()
    }
}

/// Open the projection, migrate it, evict, and only then release it.
pub fn prepare_cache(data_dir: PathBuf, publish: PhasePublisher) -> ReadyCache {
    let db = data_dir.join("workspace-cache.db");

    // A-STATE keeps this separate from `session.json`: the two have different lifetimes, and
    // losing interface state because a disposable cache was cleared would be a defect.
    let cache = match SqliteWorkspaceCache::open(&db) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            // The projection could not even be opened. Discard and try once: it is reproducible
            // by definition, so there is nothing to lose and a corrupt file to be rid of.
            crate::logging::warn(&format!("reopening the cache after {e}"));
            let _ = migrate::discard(&db);
            Arc::new(
                SqliteWorkspaceCache::open(&db)
                    .unwrap_or_else(|e| panic!("cannot create a workspace cache at {db:?}: {e}")),
            )
        }
    };

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let rebuild = {
        let db = db.clone();
        Arc::new(move || migrate::discard(&db).map_err(|e| e.to_string()))
    };

    let report = MaintainCache::new(
        cache.clone(),
        clock,
        publish,
        RetentionWindow::DAYS_14,
        rebuild,
        schema::CURRENT_VERSION,
    )
    .run();

    // A rebuild deleted the file underneath the handle, so the schema has to be laid down again.
    if report.rebuilt {
        if let Ok(fresh) = SqliteWorkspaceCache::open(&db) {
            let _ = fresh.migrate_to(schema::CURRENT_VERSION, &mut |_| {});
            return ReadyCache {
                cache: Arc::new(fresh),
                report,
            };
        }
    }

    ReadyCache { cache, report }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cache::MaintenancePhase;
    use std::sync::Mutex;

    #[test]
    fn a_fresh_data_directory_yields_a_usable_cache_at_the_current_schema() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let s = seen.clone();
            Arc::new(move |p: MaintenancePhase| s.lock().unwrap().push(p))
        };
        let ready = prepare_cache(dir.path().to_path_buf(), sink);
        assert_eq!(
            ready.get().schema_version().unwrap(),
            schema::CURRENT_VERSION,
            "the cache a provider receives is always at the schema this build reads (FR-018c)"
        );
        assert_eq!(
            seen.lock().unwrap().last(),
            Some(&MaintenancePhase::Ready),
            "and maintenance ran to completion first"
        );
    }

    #[test]
    fn a_corrupt_projection_is_rebuilt_rather_than_refusing_to_launch() {
        let dir = tempfile::tempdir().unwrap();
        // Not a database at all.
        std::fs::write(dir.path().join("workspace-cache.db"), b"this is not sqlite").unwrap();

        let ready = prepare_cache(dir.path().to_path_buf(), Arc::new(|_| {}));
        assert_eq!(
            ready.get().schema_version().unwrap(),
            schema::CURRENT_VERSION,
            "the cache is a disposable projection: rebuilding costs a refetch, and refusing to \
             launch costs the developer their session"
        );
    }

    #[test]
    fn eviction_runs_once_during_preparation_and_not_again() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let s = seen.clone();
            Arc::new(move |p: MaintenancePhase| s.lock().unwrap().push(p))
        };
        let _ready = prepare_cache(dir.path().to_path_buf(), sink);
        let evictions = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|p| matches!(p, MaintenancePhase::Evicting))
            .count();
        assert_eq!(
            evictions, 1,
            "once at startup, before any workspace opens, so deleting never competes with a read \
             (FR-026a)"
        );
    }
}
