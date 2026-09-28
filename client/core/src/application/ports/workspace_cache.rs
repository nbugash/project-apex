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

/// Work saved while disconnected, and everything needed to reconcile it (A-PENDING).
///
/// **Self-contained on purpose.** It carries the base *content*, not a reference to it, so a merge
/// never depends on a cache entry that eviction may have removed or a refetch overwritten. That
/// makes this the durable fact of offline editing: every reconciliation outcome is a statement
/// about one attempt on one of these, and the row outlives every attempt but a confirmed write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEdit {
    /// What the developer saved while offline.
    pub content: Vec<u8>,
    /// The content this was derived from, and its digest. `None` for a file created offline, which
    /// has nothing to differ from. The two move together: one without the other is a malformed row
    /// and the reader treats the file as unmergeable rather than merging against half a base.
    pub base: Option<(Vec<u8>, Sha256)>,
    /// Whether the client holds this file as text it can merge. `false` when it was never decoded
    /// as UTF-8 or it exceeds the editor's limit; such a file always prompts (FR-025a).
    pub mergeable: bool,
    /// Unix seconds, for ordering the reconciliation report. Never for deciding anything.
    pub retained_at: i64,
}

/// Progress callback, invoked at least once per second while a phase runs (FR-018a).
pub type PhaseSink<'a> = &'a mut dyn FnMut(MaintenancePhase);

/// The projection. Synchronous: the adapter crosses `spawn_blocking` at its own boundary, which
/// keeps the blocking call off a runtime worker without putting a channel in the middle.
/// One workspace's git state, as the tree and the status bar read it.
///
/// Held together because it is replaced together. A branch stored apart from the changes it
/// describes could be updated while they were not, and the status bar would name a branch the
/// marked files no longer belong to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitProjection {
    pub branch: apex_protocol::wire::BranchPosition,
    pub changes: Vec<apex_protocol::wire::GitChange>,
}

pub trait WorkspaceCache: Send + Sync {
    // ---- registration ----
    fn register(&self, ws: &Workspace, now: i64) -> CacheResult<Attachment>;
    fn forget(&self, ws: &WorkspaceId) -> CacheResult<()>;

    /// One workspace as it was registered, or `None` if this client has never seen it.
    ///
    /// Needed because a restored session carries an identity and a display name but **not the
    /// root path**, and the engine has to be told the path again: it exits when nothing is left
    /// to preserve (A-ENGINELIFE), so a relaunch commonly meets an engine that has never heard
    /// of this workspace.
    fn workspace(&self, ws: &WorkspaceId) -> CacheResult<Option<Workspace>>;

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

    /// Mark a tree region as needing re-reading before it is trusted.
    ///
    /// Distinct from content validity, which is a hash comparison. A stale region is one the
    /// client must re-query as the developer navigates into it; the blobs beneath it are
    /// untouched and each still proves itself (FR-017, FR-018, FR-026).
    fn mark_stale(&self, ws: &WorkspaceId, region: &RelPath) -> CacheResult<()>;

    /// Note that an event says this file changed.
    ///
    /// A flag **beside** validity, never a validity state. `Validity` has one constructor and
    /// it takes two hashes, so nothing but a hash comparison can declare content valid; an
    /// event is not a hash. The blob stays, stays servable offline, and the existing check
    /// settles it on next use (FR-019, FR-019a, A-UNPROVEN).
    fn mark_unproven(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<()>;

    /// Settle the doubt, once a hash comparison has agreed.
    ///
    /// Keyed by `file_id` rather than by path, because by the time the comparison runs the
    /// caller holds the entry and a path lookup would be a second chance to get it wrong.
    fn clear_unproven(&self, file_id: &FileId) -> CacheResult<()>;

    /// Rewrite a renamed directory and everything beneath it, in one transaction.
    ///
    /// Returns the number of rows rewritten, which is what a test asserts the separator
    /// boundary against: renaming `src` must rewrite `src` and `src/...` and leave
    /// `src-generated` untouched, and a count is how that stops being a spot check.
    fn rename_subtree(&self, ws: &WorkspaceId, from: &RelPath, to: &RelPath) -> CacheResult<usize>;

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

    /// Replace this workspace's git state **wholesale**, in one transaction.
    ///
    /// Wholesale rather than a delta, because git status is a whole answer: a path that has
    /// stopped differing is reported by its absence, and merging would leave it marked forever.
    /// One transaction, because a half-applied replacement is a tree that marks some files from
    /// the new state and some from the old, with nothing to say which (FR-009).
    ///
    /// **It must not touch cached content or its hashes** (§5.3, FR-010). Git status says what
    /// differs from the repository, which is not a statement about whether a cached copy still
    /// matches the host.
    fn replace_git_status(&self, ws: &WorkspaceId, git: &GitProjection) -> CacheResult<()>;

    /// What was last applied. Empty for a workspace with no git state, which is also what a
    /// workspace that is not a repository has -- the two are indistinguishable here, and
    /// deliberately so (FR-027).
    fn git_status(&self, ws: &WorkspaceId) -> CacheResult<GitProjection>;

    // ---- offline work (F012) ----
    /// Hold a saved offline edit, replacing any the path already carries.
    ///
    /// Replacing, not accumulating: a second offline save of one file supersedes the first, and the
    /// base it was derived from does **not** move (FR-011b). The implementation preserves the
    /// stored base on replacement, because re-deriving it from the new local content would make
    /// the eventual merge compare local against local — a clean merge that is wrong, which no
    /// assertion about success would notice.
    fn retain_edit(&self, ws: &WorkspaceId, path: &RelPath, edit: &PendingEdit) -> CacheResult<()>;

    /// Every path in this workspace carrying work the host has not seen.
    ///
    /// Scoped to one workspace. The table is keyed `(workspace_id, relative_path)` precisely so
    /// this can be, and a query that forgot the first half of the key would still return plausible
    /// rows — from somebody else's workspace.
    fn pending_edits(&self, ws: &WorkspaceId) -> CacheResult<Vec<(RelPath, PendingEdit)>>;

    /// Drop one path's retained work.
    ///
    /// **Only inside the transaction that commits the host write.** Called alone, it is how work is
    /// lost: every other outcome of a reconciliation attempt — a conflict, a refused stale write, a
    /// connection that dropped — must leave the row alone, which is what makes FR-022 true by
    /// construction rather than by care.
    fn forget_pending(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<()>;

    fn schema_version(&self) -> CacheResult<u32>;

    /// Migrate to `target`, one transaction per step, publishing progress throughout.
    fn migrate_to(&self, target: u32, progress: PhaseSink<'_>) -> Result<(), MigrationFailure>;
}
