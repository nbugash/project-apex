//! An in-memory `WorkspaceCache`.
//!
//! Reproduces C1-C6 from contracts/cache.md, and — the part that matters — **fails on demand**.
//! A fake that cannot fail tests only the happy path, and FR-034 does not live there.

#![allow(dead_code)]

use apex_shell::application::ports::workspace_cache::{
    Attachment, CacheError, CacheResult, EvictionReport, MigrationFailure, PhaseSink, StoreOutcome,
    WorkspaceCache,
};
use apex_shell::domain::cache::CacheEntry;
use apex_shell::domain::workspace::{
    EntryKind, FileId, FsEntry, RelPath, Sha256, Workspace, WorkspaceId,
};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// A-CACHECAP, mirrored so the fake refuses what the real store refuses.
pub const CACHE_MAX_BYTES: u64 = 8 * 1024 * 1024;

struct Row {
    file_id: FileId,
    entry: FsEntry,
    parent: String,
    hash: Option<Sha256>,
    bytes: Option<Vec<u8>>,
    last_accessed_at: Option<i64>,
    /// Schema v2. A property of the tree row: re-query before trusting.
    stale: bool,
    /// Schema v2. A flag beside validity, never a validity state.
    unproven: bool,
}

impl Row {
    fn new(entry: FsEntry, parent: &str) -> Self {
        Self {
            file_id: FileId::new(),
            entry,
            parent: parent.to_string(),
            hash: None,
            bytes: None,
            last_accessed_at: None,
            stale: false,
            unproven: false,
        }
    }
}

#[derive(Default)]
pub struct InMemoryCache {
    workspaces: Mutex<BTreeMap<String, Workspace>>,
    /// (workspace, relative path) -> row.
    rows: Mutex<BTreeMap<(String, String), Row>>,
    /// When set, every write fails with this.
    write_fails: Mutex<Option<CacheError>>,
    version: Mutex<u32>,
    /// When set, migration fails deterministically — the case retrying cannot fix.
    migration_fails: Mutex<bool>,
}

impl InMemoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Make every content write fail. For FR-034: a full disk must not fail the read.
    pub fn fail_writes(&self, why: &str) {
        *self.write_fails.lock().unwrap() = Some(CacheError::Store(why.into()));
    }

    pub fn set_version(&self, v: u32) {
        *self.version.lock().unwrap() = v;
    }

    pub fn fail_migration(&self) {
        *self.migration_fails.lock().unwrap() = true;
    }

    /// Age a file's access time, so retention is a matter of arithmetic.
    pub fn set_accessed(&self, ws: &WorkspaceId, path: &RelPath, at: i64) {
        if let Some(r) = self
            .rows
            .lock()
            .unwrap()
            .get_mut(&(ws.0.clone(), path.as_str().into()))
        {
            r.last_accessed_at = Some(at);
        }
    }

    pub fn is_cached(&self, ws: &WorkspaceId, path: &RelPath) -> bool {
        self.rows
            .lock()
            .unwrap()
            .get(&(ws.0.clone(), path.as_str().to_string()))
            .map(|r| r.bytes.is_some())
            .unwrap_or(false)
    }

    pub fn is_listed(&self, ws: &WorkspaceId, path: &RelPath) -> bool {
        self.rows
            .lock()
            .unwrap()
            .contains_key(&(ws.0.clone(), path.as_str().to_string()))
    }

    pub fn file_id_of(&self, ws: &WorkspaceId, path: &RelPath) -> Option<FileId> {
        self.rows
            .lock()
            .unwrap()
            .get(&(ws.0.clone(), path.as_str().to_string()))
            .map(|r| r.file_id.clone())
    }
}

impl WorkspaceCache for InMemoryCache {
    fn register(&self, ws: &Workspace, _now: i64) -> CacheResult<Attachment> {
        let mut w = self.workspaces.lock().unwrap();
        let existed = w.contains_key(&ws.id.0);
        w.insert(ws.id.0.clone(), ws.clone());
        Ok(if existed {
            Attachment::Attached
        } else {
            Attachment::Created
        })
    }

    fn forget(&self, ws: &WorkspaceId) -> CacheResult<()> {
        self.workspaces.lock().unwrap().remove(&ws.0);
        self.rows.lock().unwrap().retain(|(w, _), _| w != &ws.0);
        Ok(())
    }

    fn list_children(&self, ws: &WorkspaceId, parent: &RelPath) -> CacheResult<Vec<FsEntry>> {
        let rows = self.rows.lock().unwrap();
        let mut out: Vec<FsEntry> = rows
            .iter()
            .filter(|((w, _), r)| w == &ws.0 && r.parent == parent.as_str())
            .map(|(_, r)| r.entry.clone())
            .collect();
        out.sort_by(FsEntry::listing_order);
        Ok(out)
    }

    fn put_listing(
        &self,
        ws: &WorkspaceId,
        parent: &RelPath,
        entries: &[FsEntry],
    ) -> CacheResult<()> {
        let mut rows = self.rows.lock().unwrap();
        let keep: std::collections::HashSet<&str> =
            entries.iter().map(|e| e.name.as_str()).collect();
        // A vanished name takes its content with it. This cannot recognise a rename (C9).
        rows.retain(|(w, _), r| {
            !(w == &ws.0 && r.parent == parent.as_str() && !keep.contains(r.entry.name.as_str()))
        });
        for e in entries {
            let rel = format!(
                "{}/{}",
                if parent.is_root() {
                    ""
                } else {
                    parent.as_str()
                },
                e.name
            );
            let key = (ws.0.clone(), rel);
            match rows.get_mut(&key) {
                Some(r) => r.entry = e.clone(),
                None => {
                    rows.insert(key, Row::new(e.clone(), parent.as_str()));
                }
            }
        }
        Ok(())
    }

    fn lookup(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<Option<CacheEntry>> {
        let rows = self.rows.lock().unwrap();
        let Some(r) = rows.get(&(ws.0.clone(), path.as_str().to_string())) else {
            return Ok(None);
        };
        let (Some(hash), Some(bytes)) = (&r.hash, &r.bytes) else {
            return Ok(None);
        };
        Ok(Some(CacheEntry {
            file_id: r.file_id.clone(),
            path: path.clone(),
            hash: hash.clone(),
            bytes: bytes.clone(),
            last_accessed_at: r.last_accessed_at.unwrap_or(0),
        }))
    }

    fn file_id(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<Option<FileId>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&(ws.0.clone(), path.as_str().to_string()))
            .map(|r| r.file_id.clone()))
    }

    fn put_content(&self, file_id: &FileId, bytes: &[u8], hash: &Sha256, now: i64) -> StoreOutcome {
        if bytes.len() as u64 > CACHE_MAX_BYTES {
            return StoreOutcome::NotEligible {
                size: bytes.len() as u64,
            };
        }
        if let Some(e) = self.write_fails.lock().unwrap().clone() {
            return StoreOutcome::Failed(e);
        }
        let mut rows = self.rows.lock().unwrap();
        for r in rows.values_mut() {
            if &r.file_id == file_id {
                r.hash = Some(hash.clone());
                r.bytes = Some(bytes.to_vec());
                r.last_accessed_at = Some(now);
                return StoreOutcome::Stored;
            }
        }
        StoreOutcome::Failed(CacheError::Store("no such file_id".into()))
    }

    fn touch(&self, file_id: &FileId, now: i64) -> StoreOutcome {
        let mut rows = self.rows.lock().unwrap();
        for r in rows.values_mut() {
            if &r.file_id == file_id {
                r.last_accessed_at = Some(now);
                return StoreOutcome::Stored;
            }
        }
        StoreOutcome::Failed(CacheError::Store("no such file_id".into()))
    }

    fn rename(&self, file_id: &FileId, to: &RelPath) -> CacheResult<()> {
        let mut rows = self.rows.lock().unwrap();
        let Some((key, _)) = rows
            .iter()
            .find(|(_, r)| &r.file_id == file_id)
            .map(|(k, _)| (k.clone(), ()))
        else {
            return Err(CacheError::Store("no such file_id".into()));
        };
        let mut row = rows.remove(&key).expect("just found");
        row.entry.name = to.name().to_string();
        row.parent = to
            .parent()
            .unwrap_or_else(RelPath::root)
            .as_str()
            .to_string();
        // Content untouched: that is the whole point of an opaque file_id (FR-022).
        rows.insert((key.0, to.as_str().to_string()), row);
        Ok(())
    }

    fn mark_stale(&self, ws: &WorkspaceId, region: &RelPath) -> CacheResult<()> {
        let mut rows = self.rows.lock().unwrap();
        let whole = region.as_str() == "/" || region.as_str().is_empty();
        let prefix = format!("{}/", region.as_str());
        for ((w, path), row) in rows.iter_mut() {
            if w != &ws.0 {
                continue;
            }
            // The separator matters as much here as in SQL: marking `src` must not reach
            // `src-generated`, and a fake that is laxer than the real thing hides the bug.
            if whole || path == region.as_str() || path.starts_with(&prefix) {
                row.stale = true;
            }
        }
        Ok(())
    }

    fn mark_unproven(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<()> {
        let mut rows = self.rows.lock().unwrap();
        if let Some(row) = rows.get_mut(&(ws.0.clone(), path.as_str().to_string())) {
            row.unproven = true;
        }
        Ok(())
    }

    fn clear_unproven(&self, file_id: &FileId) -> CacheResult<()> {
        let mut rows = self.rows.lock().unwrap();
        for row in rows.values_mut() {
            if &row.file_id == file_id {
                row.unproven = false;
            }
        }
        Ok(())
    }

    fn rename_subtree(&self, ws: &WorkspaceId, from: &RelPath, to: &RelPath) -> CacheResult<usize> {
        let mut rows = self.rows.lock().unwrap();
        let from_prefix = format!("{}/", from.as_str());
        let moving: Vec<(String, String)> = rows
            .keys()
            .filter(|(w, p)| w == &ws.0 && (p == from.as_str() || p.starts_with(&from_prefix)))
            .cloned()
            .collect();
        let count = moving.len();
        for key in moving {
            let mut row = rows.remove(&key).expect("just listed");
            let rewritten = if key.1 == from.as_str() {
                to.as_str().to_string()
            } else {
                format!("{}{}", to.as_str(), &key.1[from.as_str().len()..])
            };
            let parsed = RelPath::parse(&rewritten).expect("a rewritten path is still a path");
            row.entry.name = parsed.name().to_string();
            row.parent = parsed
                .parent()
                .unwrap_or_else(RelPath::root)
                .as_str()
                .to_string();
            // Content untouched: a rename moves the entry, it does not replace the file.
            rows.insert((key.0, rewritten), row);
        }
        Ok(count)
    }

    fn search_paths(
        &self,
        ws: &WorkspaceId,
        fragment: &str,
        limit: u32,
    ) -> CacheResult<Vec<RelPath>> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .keys()
            .filter(|(w, p)| w == &ws.0 && p.contains(fragment))
            .filter_map(|(_, p)| RelPath::parse(p).ok())
            .take(limit as usize)
            .collect())
    }

    fn evict(&self, before: i64) -> CacheResult<EvictionReport> {
        let mut rows = self.rows.lock().unwrap();
        let mut report = EvictionReport::default();
        for r in rows.values_mut() {
            if r.last_accessed_at.is_some_and(|a| a < before) && r.bytes.is_some() {
                report.blobs_removed += 1;
                report.bytes_reclaimed += r.bytes.as_ref().unwrap().len() as u64;
                // Content only. The row stays, so the tree stays navigable (C4, FR-027).
                r.bytes = None;
                r.hash = None;
            }
        }
        Ok(report)
    }

    fn schema_version(&self) -> CacheResult<u32> {
        Ok(*self.version.lock().unwrap())
    }

    fn migrate_to(&self, target: u32, progress: PhaseSink<'_>) -> Result<(), MigrationFailure> {
        use apex_shell::domain::cache::MaintenancePhase;
        let from = *self.version.lock().unwrap();
        if from > target {
            return Err(MigrationFailure::FromTheFuture {
                found: from,
                expected: target,
            });
        }
        for step in (from + 1)..=target {
            progress(MaintenancePhase::Migrating { from, to: target });
            if *self.migration_fails.lock().unwrap() {
                return Err(MigrationFailure::Step {
                    version: step,
                    why: "told to fail".into(),
                });
            }
        }
        *self.version.lock().unwrap() = target;
        Ok(())
    }
}

/// Seed rows at the given paths, for tests that care about paths rather than content.
pub fn seed(cache: &InMemoryCache, ws: &WorkspaceId, paths: &[&str]) {
    let mut rows = cache.rows.lock().unwrap();
    for path in paths {
        let parsed = RelPath::parse(path).expect("a seed path is a path");
        let entry = FsEntry {
            name: parsed.name().to_string(),
            kind: if path.ends_with(".rs") || path.ends_with(".txt") {
                EntryKind::File
            } else {
                EntryKind::Directory
            },
            size: 1,
            modified: 0,
        };
        let parent = parsed
            .parent()
            .unwrap_or_else(RelPath::root)
            .as_str()
            .to_string();
        rows.insert(
            (ws.0.clone(), parsed.as_str().to_string()),
            Row::new(entry, &parent),
        );
    }
}

/// Every path this workspace holds, sorted.
pub fn paths(cache: &InMemoryCache, ws: &WorkspaceId) -> Vec<String> {
    let rows = cache.rows.lock().unwrap();
    let mut out: Vec<String> = rows
        .keys()
        .filter(|(w, _)| w == &ws.0)
        .map(|(_, p)| p.clone())
        .collect();
    out.sort();
    out
}
