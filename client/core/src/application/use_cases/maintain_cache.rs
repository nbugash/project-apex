//! Migrate, then evict, once, before anything reads.
//!
//! The ordering is the requirement, not an implementation detail: FR-018c forbids reading a
//! projection with a schema it was not written for *at any point*, and FR-026a forbids evicting
//! while a workspace is open. Both are satisfied by this running to completion before any provider
//! exists, which the composition root enforces.

use crate::application::ports::clock::Clock;
use crate::application::ports::workspace_cache::{MigrationFailure, WorkspaceCache};
use crate::domain::cache::{MaintenancePhase, RetentionWindow};
use std::sync::Arc;

/// Where a phase is published, at least once per second while one runs (FR-018a).
pub type PhasePublisher = Arc<dyn Fn(MaintenancePhase) + Send + Sync>;

/// What maintenance did, for the report the developer sees and the tests assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceReport {
    pub from_version: u32,
    pub to_version: u32,
    /// True when a failed or future-dated migration forced a rebuild (FR-018b).
    pub rebuilt: bool,
    pub blobs_evicted: u64,
    pub bytes_reclaimed: u64,
}

/// What to do when a migration cannot complete.
///
/// Injected rather than performed here, because discarding is a filesystem act and this use case
/// holds ports only. The composition root supplies one that deletes the database and its `-wal`
/// and `-shm` companions.
pub type Rebuild = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

pub struct MaintainCache {
    cache: Arc<dyn WorkspaceCache>,
    clock: Arc<dyn Clock>,
    publish: PhasePublisher,
    retention: RetentionWindow,
    rebuild: Rebuild,
    target_version: u32,
}

impl MaintainCache {
    pub fn new(
        cache: Arc<dyn WorkspaceCache>,
        clock: Arc<dyn Clock>,
        publish: PhasePublisher,
        retention: RetentionWindow,
        rebuild: Rebuild,
        target_version: u32,
    ) -> Self {
        Self {
            cache,
            clock,
            publish,
            retention,
            rebuild,
            target_version,
        }
    }

    /// Run maintenance to completion.
    ///
    /// **Never returns an error.** A failure becomes a rebuild and is reported (FR-018b):
    /// refusing to launch over a cache the specification itself calls a disposable projection is
    /// not a defensible outcome, and it would strand a developer behind data they never needed.
    pub fn run(&self) -> MaintenanceReport {
        (self.publish)(MaintenancePhase::Checking);
        let from = self.cache.schema_version().unwrap_or(0);

        let mut rebuilt = false;
        let publish = self.publish.clone();
        let mut on_phase = move |p: MaintenancePhase| publish(p);

        match self.cache.migrate_to(self.target_version, &mut on_phase) {
            Ok(()) => {}
            Err(failure) => {
                // Two cases, one treatment. A step that fails deterministically cannot be fixed
                // by retrying, and a projection written by a newer build cannot be read by this
                // one — refusing to launch after a downgrade would be the same indefensible
                // outcome as refusing after a failure.
                let why = match &failure {
                    MigrationFailure::Step { version, why } => {
                        format!("migration step {version} failed: {why}")
                    }
                    MigrationFailure::FromTheFuture { found, expected } => {
                        format!("cache is v{found}; this build reads v{expected}")
                    }
                };
                crate::logging::warn(&format!("rebuilding the cache: {why}"));
                (self.publish)(MaintenancePhase::Rebuilding);
                if let Err(e) = (self.rebuild)() {
                    crate::logging::warn(&format!("could not rebuild the cache: {e}"));
                }
                rebuilt = true;
            }
        }

        (self.publish)(MaintenancePhase::Evicting);
        let cutoff = self.retention.cutoff(self.clock.now());
        let report = self.cache.evict(cutoff).unwrap_or_default();

        (self.publish)(MaintenancePhase::Ready);
        MaintenanceReport {
            from_version: from,
            to_version: self.target_version,
            rebuilt,
            blobs_evicted: report.blobs_removed,
            bytes_reclaimed: report.bytes_reclaimed,
        }
    }
}
