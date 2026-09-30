//! Cache what the developer is likely to want before the connection goes (§11.4, US5).
//!
//! Manifests first, then the files recent commits touched, through a caching reader sent at
//! background priority so it cannot delay anything the developer asked for (FR-030, §4.6).
//!
//! **It stops rather than evicts** (FR-029a). Before each fetch it checks the budget
//! (A-PREFETCHCAP), and when the next file would take cached content over it, it stops and says so.
//! The cache itself never evicts by size -- §5.5's policy is age alone and is unchanged -- so the
//! budget is prefetch's own limit on speculative content, and "evicts nothing" holds by
//! construction rather than by care.
//!
//! **It lists before it fetches.** The caching layer stores content only for a file the tree has a
//! row for, and a recent-commit path is usually in a folder nobody expanded. Listing its ancestors
//! is what makes the fetch stick, and is also what makes the file reachable in the offline tree.

use crate::application::ports::connection::ConnectionStatusSource;
use crate::application::ports::git_provider::GitProvider;
use crate::application::ports::workspace_cache::WorkspaceCache;
use crate::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use crate::application::use_cases::cached_workspace::CachedWorkspace;
use crate::domain::connection::ConnectionState;
use crate::domain::workspace::{EntryKind, PageRequest, RelPath, WorkspaceId};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A-PREFETCHCAP: prefetch stops before cached content would exceed this. 32 files at A-CACHECAP's
/// per-file limit, and thousands of ordinary source files.
pub const PREFETCH_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// The largest response a prefetch read asks for. One pipe is one queue, so this is how long an
/// interactive reply can wait behind prefetch: one frame of this size, not one whole file (SC-009 as
/// amended, §4.6).
pub const PREFETCH_CHUNK_BYTES: u64 = 64 * 1024;

/// §11.4's manifests, looked for at the workspace root.
pub const MANIFESTS: [&str; 5] = [
    "go.mod",
    "Cargo.toml",
    "package.json",
    "mix.exs",
    "pyproject.toml",
];

/// What one run achieved. Returned rather than logged: SC-010 is measured against what prefetch says
/// it fetched and SC-010a against whether it stopped, and a run that only logged would leave both
/// unmeasurable while looking finished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrefetchReport {
    /// Files now cached that were not before this run.
    pub fetched: usize,
    /// The budget was reached. An ordinary outcome, never a failure (FR-029a).
    pub stopped_at_budget: bool,
}

pub struct Prefetch {
    reader: Arc<CachedWorkspace>,
    git: Arc<dyn GitProvider>,
    cache: Arc<dyn WorkspaceCache>,
    connection: Arc<dyn ConnectionStatusSource>,
    budget: u64,
    /// Two triggers exist (FR-029b); a run already going makes the second a no-op rather than a
    /// second reader of the same paths.
    running: AtomicBool,
}

impl Prefetch {
    /// `reader` must be a caching provider sent at background priority; see the module note.
    pub fn new(
        reader: Arc<CachedWorkspace>,
        git: Arc<dyn GitProvider>,
        cache: Arc<dyn WorkspaceCache>,
        connection: Arc<dyn ConnectionStatusSource>,
    ) -> Self {
        Self {
            reader,
            git,
            cache,
            connection,
            budget: PREFETCH_BUDGET_BYTES,
            running: AtomicBool::new(false),
        }
    }

    pub fn with_budget(mut self, bytes: u64) -> Self {
        self.budget = bytes;
        self
    }

    /// One run for `ws`. Nothing at all while disconnected: FR-029b's two triggers both imply a
    /// connection, and a run that asked a host that is not there would only produce failures.
    pub async fn run(&self, ws: &WorkspaceId) -> PrefetchReport {
        if !matches!(self.connection.current(), ConnectionState::Connected) {
            return PrefetchReport::default();
        }
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return PrefetchReport::default();
        }
        let report = self.run_once(ws).await;
        self.running.store(false, Ordering::Release);
        report
    }

    async fn run_once(&self, ws: &WorkspaceId) -> PrefetchReport {
        let mut report = PrefetchReport::default();
        let Some(candidates) = self.candidates(ws).await else {
            return report;
        };
        for path in candidates {
            if matches!(self.cache.lookup(ws, &path), Ok(Some(_))) {
                continue;
            }
            let size = match self.listed_size(ws, &path).await {
                Listed::File(size) => size,
                Listed::NotAFile => continue,
                Listed::Offline => break,
            };
            let used = self.cache.cached_bytes().unwrap_or(u64::MAX);
            if used.saturating_add(size) > self.budget {
                report.stopped_at_budget = true;
                break;
            }
            match self
                .reader
                .cache_in_ranges(ws, &path, PREFETCH_CHUNK_BYTES)
                .await
            {
                // Counted only if it is now cached: the report is a claim about what is readable
                // offline, and a file above A-CACHECAP, or one that changed mid-read, was not kept.
                Ok(true) => report.fetched += 1,
                Ok(false) => {}
                // The connection went. What completed is cached whole and usable; the read that did
                // not complete stored nothing, because content is stored only once it has arrived
                // (FR-031).
                Err(ProviderError::Offline) => break,
                Err(_) => {}
            }
        }
        report
    }

    /// Manifests at the root, then recent-commit paths, deduplicated. `None` if the root cannot be
    /// listed because the connection is gone.
    async fn candidates(&self, ws: &WorkspaceId) -> Option<Vec<RelPath>> {
        let root = RelPath::root();
        if let Err(ProviderError::Offline) = self
            .reader
            .read_directory(ws, &root, PageRequest::default())
            .await
        {
            return None;
        }
        let children = self.cache.list_children(ws, &root).unwrap_or_default();
        let mut out: Vec<RelPath> = MANIFESTS
            .iter()
            .filter(|m| {
                children
                    .iter()
                    .any(|e| e.name == **m && e.kind == EntryKind::File)
            })
            .filter_map(|m| root.join(m).ok())
            .collect();
        // A git that cannot answer -- no repository, no git, a transport hiccup -- leaves the
        // manifests, which is a complete outcome for a plain directory (FR-032).
        let recent = self.git.recently_changed(ws).await.unwrap_or_default();
        for raw in recent {
            // From the engine, so untrusted (Principle VI): refused rather than repaired.
            if let Ok(p) = RelPath::parse(&raw) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        Some(out)
    }

    /// The file's size from its listing, listing each ancestor the tree has not listed yet.
    async fn listed_size(&self, ws: &WorkspaceId, path: &RelPath) -> Listed {
        let mut chain = Vec::new();
        let mut at = path.parent();
        while let Some(dir) = at {
            at = dir.parent();
            chain.push(dir);
        }
        chain.reverse();
        let mut below = chain.iter().skip(1).chain(std::iter::once(path));
        for dir in &chain {
            let next = below.next().expect("one child per ancestor");
            let listed = self
                .cache
                .list_children(ws, dir)
                .unwrap_or_default()
                .iter()
                .any(|e| e.name == next.name());
            if listed {
                continue;
            }
            match self
                .reader
                .read_directory(ws, dir, PageRequest::default())
                .await
            {
                Ok(_) => {}
                Err(ProviderError::Offline) => return Listed::Offline,
                Err(_) => return Listed::NotAFile,
            }
        }
        let Some(parent) = path.parent() else {
            return Listed::NotAFile;
        };
        match self
            .cache
            .list_children(ws, &parent)
            .unwrap_or_default()
            .into_iter()
            .find(|e| e.name == path.name())
        {
            Some(e) if e.kind == EntryKind::File => Listed::File(e.size),
            _ => Listed::NotAFile,
        }
    }
}

enum Listed {
    File(u64),
    NotAFile,
    Offline,
}
