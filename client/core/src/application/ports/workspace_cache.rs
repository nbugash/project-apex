//! Outbound port: the local projection, as a capability rather than a technology.
//!
//! `WorkspaceCache`, not `SqliteWorkspaceStore` (Principle VIII). Guarantees C1-C9 are stated in
//! specs/005-workspace-cache/contracts/cache.md; §5.2 owns the schema this projects onto.

use crate::domain::cache::{CacheEntry, MaintenancePhase};
use crate::domain::workspace::{FileId, FsEntry, RelPath, Sha256, Workspace, WorkspaceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    /// The store rejected the operation — locked, full, or corrupt.
    Store(String),
    /// The schema on disk is not the one this build reads.
    SchemaMismatch { found: u32, expected: u32 },
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(why) => write!(f, "cache store: {why}"),
            Self::SchemaMismatch { found, expected } => {
                write!(f, "cache schema v{found}, this build reads v{expected}")
            }
        }
    }
}

pub type CacheResult<T> = Result<T, CacheError>;

/// What happened to a write, as a value the caller is permitted to ignore.
///
/// **Deliberately not a `Result`.** FR-034 says a caching failure must never fail the read that
/// prompted it, and a `Result` invites `?` — which is how that requirement gets violated by a
/// reflex rather than by a decision. Making the type carry the rule means the compiler does not
/// offer the shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a caching outcome is recorded or deliberately ignored, never silently dropped"]
pub enum StoreOutcome {
    Stored,
    /// Above A-CACHECAP's 8 MiB limit. Distinguishable from a failure: nothing went wrong, and
    /// the file is simply never available offline.
    NotEligible {
        size: u64,
    },
    Failed(CacheError),
}

/// Whether registering attached to an existing projection or created one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attachment {
    Created,
    /// FR-011: re-opening attaches rather than building a second projection.
    Attached,
}

/// What eviction reclaimed, for the report the developer sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvictionReport {
    pub blobs_removed: u64,
    pub bytes_reclaimed: u64,
}

/// Why a migration could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationFailure {
    /// A step failed. The transaction rolled back, so the file is still at its previous version.
    Step { version: u32, why: String },
    /// Written by a newer build than this one.
    FromTheFuture { found: u32, expected: u32 },
}

/// Progress callback, invoked at least once per second while a phase runs (FR-018a).
pub type PhaseSink<'a> = &'a mut dyn FnMut(MaintenancePhase);

/// The projection. Synchronous: the adapter crosses `spawn_blocking` at its own boundary, which
/// keeps the blocking call off a runtime worker without putting a channel in the middle.
pub trait WorkspaceCache: Send + Sync {
    // ---- registration ----
    fn register(&self, ws: &Workspace, now: i64) -> CacheResult<Attachment>;
    fn forget(&self, ws: &WorkspaceId) -> CacheResult<()>;

    // ---- tree ----
    /// One indexed query, no join. Ordered as §5.4's sidebar query orders.
    fn list_children(&self, ws: &WorkspaceId, parent: &RelPath) -> CacheResult<Vec<FsEntry>>;

    /// Replace a parent's children atomically.
    ///
    /// `file_id` is preserved for entries that remain **under the same name**, so their content
    /// survives. It cannot recognise a rename: a re-listing shows one name gone and another
    /// present with nothing linking them, and the vanished entry's content cascades away (C9,
    /// FR-022a). The file is re-cached on next open and stays listed throughout (FR-022b).
    fn put_listing(
        &self,
        ws: &WorkspaceId,
        parent: &RelPath,
        entries: &[FsEntry],
    ) -> CacheResult<()>;

    // ---- content ----
    fn lookup(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<Option<CacheEntry>>;

    /// The identity of a **listed** file, cached or not.
    ///
    /// `put_content` takes a `FileId`, and `lookup` only yields one for a file that already has
    /// content — so without this there is no way to cache a file for the first time. The gap was
    /// found by writing the contract suite: the port declared an operation nothing could reach,
    /// which is the same shape as F001's `withdraw`, unreachable because nothing returned the id
    /// it required.
    fn file_id(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<Option<FileId>>;

    /// Hash first, compress second (C1, §5.6), so the stored digest is over decompressed bytes
    /// and compares directly with the engine's.
    fn put_content(&self, file_id: &FileId, bytes: &[u8], hash: &Sha256, now: i64) -> StoreOutcome;

    /// Record an access, so retention measures use rather than age (FR-028).
    fn touch(&self, file_id: &FileId, now: i64) -> StoreOutcome;

    /// Move a file's path, leaving `file_contents` untouched (FR-022).
    ///
    /// The operation A-B5's opaque `file_id` exists for. Its first caller is F006's write path:
    /// nothing in F003 invokes it, because nothing here performs a rename.
    fn rename(&self, file_id: &FileId, to: &RelPath) -> CacheResult<()>;

    // ---- search ----
    /// `files_fts`, never a leading-wildcard `LIKE` (§5.2, §5.4). Never consults a provider (C6).
    fn search_paths(
        &self,
        ws: &WorkspaceId,
        fragment: &str,
        limit: u32,
    ) -> CacheResult<Vec<RelPath>>;

    // ---- maintenance ----
    /// Remove content only. Never removes a `files` row (C4, FR-027, §5.5).
    fn evict(&self, before: i64) -> CacheResult<EvictionReport>;

    fn schema_version(&self) -> CacheResult<u32>;

    /// Migrate to `target`, one transaction per step, publishing progress throughout.
    fn migrate_to(&self, target: u32, progress: PhaseSink<'_>) -> Result<(), MigrationFailure>;
}
