//! The §5.2 projection, over `rusqlite`.
//!
//! One connection behind a mutex. SQLite's C library is synchronous, so every call crosses into
//! `spawn_blocking` at the *application* boundary rather than here — the port is synchronous
//! (see `ports::workspace_cache`) and the caller owns the handoff, which keeps this adapter free
//! of runtime types.
//!
//! One connection rather than a pool because of what actually contends: a single-user desktop
//! application reads from one interface at a time, and the only sustained writer is maintenance,
//! which by FR-026a runs before any workspace is open. A pool would add configuration, a checkout
//! path and a failure mode in exchange for parallelism that has no second party.

pub mod migrate;
pub mod schema;

use crate::application::ports::workspace_cache::{
    Attachment, CacheError, CacheResult, EvictionReport, MigrationFailure, PendingEdit, PhaseSink,
    StoreOutcome, WorkspaceCache,
};
use crate::domain::cache::CacheEntry;
use crate::domain::workspace::{
    EntryKind, FileId, FsEntry, Location, RelPath, Sha256, Workspace, WorkspaceId,
};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct SqliteWorkspaceCache {
    conn: Mutex<Connection>,
}

fn store(e: impl std::fmt::Display) -> CacheError {
    CacheError::Store(e.to_string())
}

impl SqliteWorkspaceCache {
    /// Open (or create) the projection at `path`.
    ///
    /// Does **not** migrate: `MaintainCache` owns that, and it must run before any provider
    /// exists (FR-018c). Opening and migrating in one call would make the ordering a convention
    /// rather than a structure.
    pub fn open(path: &Path) -> CacheResult<Self> {
        let conn = Connection::open(path).map_err(store)?;
        schema::apply_pragmas(&conn).map_err(store)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// An in-memory projection, for tests that want the real SQL without a file.
    pub fn in_memory() -> CacheResult<Self> {
        let conn = Connection::open_in_memory().map_err(store)?;
        schema::apply_pragmas(&conn).map_err(store)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> CacheResult<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| CacheError::Store("poisoned".into()))?;
        f(&guard).map_err(store)
    }

    /// Total size of every stored content blob, compressed.
    ///
    /// SC-010 requires the compression ratio to be **measured and printed** rather than only
    /// compared, so the store has to be able to report it. Exposed as a number rather than by
    /// handing out a connection, which would put `rusqlite` in this adapter's public surface and
    /// let a caller reach around the port (Principle VIII).
    pub fn stored_content_bytes(&self) -> CacheResult<u64> {
        self.with(|c| {
            c.query_row(
                "SELECT coalesce(sum(length(content_blob)), 0) FROM file_contents",
                [],
                |r| r.get::<_, i64>(0),
            )
        })
        .map(|v| v as u64)
    }

    pub(crate) fn register_inner(&self, ws: &Workspace, now: i64) -> CacheResult<Attachment> {
        let (location_type, base_path, ssh_host) = match &ws.location {
            Location::Remote { host, base } => ("REMOTE", base.clone(), Some(host.clone())),
            Location::Local { base } => ("LOCAL", base.clone(), None),
        };
        self.with(|c| {
            let existing: Option<String> = c
                .query_row(
                    "SELECT workspace_id FROM workspaces WHERE workspace_id = ?1",
                    [&ws.id.0],
                    |r| r.get(0),
                )
                .ok();
            if existing.is_some() {
                // FR-011: attach to the existing projection rather than build a second one.
                c.execute(
                    "UPDATE workspaces SET name=?2, last_opened_at=?3 WHERE workspace_id=?1",
                    rusqlite::params![&ws.id.0, &ws.name, now],
                )?;
                Ok(Attachment::Attached)
            } else {
                c.execute(
                    "INSERT INTO workspaces
                       (workspace_id, name, location_type, base_path, ssh_host, last_opened_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![&ws.id.0, &ws.name, location_type, base_path, ssh_host, now],
                )?;
                Ok(Attachment::Created)
            }
        })
    }

    pub(crate) fn forget_inner(&self, ws: &WorkspaceId) -> CacheResult<()> {
        // Content and tree go together, by cascade (FR-012). The cascade depends on the
        // per-connection foreign_keys pragma, which `apply_pragmas` sets and a schema test proves.
        self.with(|c| {
            c.execute("DELETE FROM workspaces WHERE workspace_id = ?1", [&ws.0])?;
            Ok(())
        })
    }
}

/// A-CACHECAP: content above this is read but never cached, so a multi-gigabyte artifact cannot
/// be pulled into the projection whole. The honest consequence is that such a file is never
/// available offline.
pub const CACHE_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// §5.6: Zstd level 3 balances ratio against decompression speed, and decompression has to stay
/// off the interaction path.
const ZSTD_LEVEL: i32 = 3;

impl WorkspaceCache for SqliteWorkspaceCache {
    fn register(&self, ws: &Workspace, now: i64) -> CacheResult<Attachment> {
        self.register_inner(ws, now)
    }

    fn forget(&self, ws: &WorkspaceId) -> CacheResult<()> {
        self.forget_inner(ws)
    }

    fn list_children(&self, ws: &WorkspaceId, parent: &RelPath) -> CacheResult<Vec<FsEntry>> {
        // §5.4's sidebar query: one index, no join, already in the contractual order so the
        // client renders what it receives without re-sorting.
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT name, is_directory, size_bytes, remote_modified_at
                   FROM files
                  WHERE workspace_id = ?1 AND parent_path = ?2
                  ORDER BY is_directory DESC, name ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![&ws.0, parent.as_str()], |r| {
                let is_dir: i64 = r.get(1)?;
                Ok(FsEntry {
                    name: r.get(0)?,
                    kind: if is_dir != 0 {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    size: r.get::<_, i64>(2)? as u64,
                    modified: r.get(3)?,
                })
            })?;
            rows.collect()
        })
    }

    fn put_listing(
        &self,
        ws: &WorkspaceId,
        parent: &RelPath,
        entries: &[FsEntry],
    ) -> CacheResult<()> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            // Which children exist now, so survivors keep their `file_id` and their content.
            let mut existing: std::collections::HashMap<String, String> = Default::default();
            {
                let mut stmt = tx.prepare(
                    "SELECT name, file_id FROM files WHERE workspace_id = ?1 AND parent_path = ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![&ws.0, parent.as_str()], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (n, id) = row?;
                    existing.insert(n, id);
                }
            }
            let keep: std::collections::HashSet<&str> =
                entries.iter().map(|e| e.name.as_str()).collect();

            // A vanished name takes its content with it, by cascade. This cannot recognise a
            // rename: a re-listing carries no identity linking one gone name to one new name, so
            // the content is refetched on next open (C9, FR-022a) and the entry stays listed
            // throughout (FR-022b).
            for (name, id) in &existing {
                if !keep.contains(name.as_str()) {
                    tx.execute("DELETE FROM files WHERE file_id = ?1", [id])?;
                }
            }

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
                match existing.get(&e.name) {
                    Some(id) => {
                        tx.execute(
                            "UPDATE files SET is_directory=?2, size_bytes=?3, remote_modified_at=?4
                              WHERE file_id=?1",
                            rusqlite::params![
                                id,
                                matches!(e.kind, EntryKind::Directory) as i64,
                                e.size as i64,
                                e.modified
                            ],
                        )?;
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO files
                               (file_id, workspace_id, parent_path, relative_path, name,
                                is_directory, size_bytes, remote_modified_at, is_cached)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0)",
                            rusqlite::params![
                                FileId::new().0,
                                &ws.0,
                                parent.as_str(),
                                &rel,
                                &e.name,
                                matches!(e.kind, EntryKind::Directory) as i64,
                                e.size as i64,
                                e.modified
                            ],
                        )?;
                    }
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    fn lookup(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<Option<CacheEntry>> {
        // §5.4's open query: metadata and blob together, in one statement.
        self.with(|c| {
            let row = c.query_row(
                "SELECT f.file_id, f.is_cached, c.sha256_hash, c.content_blob, f.last_accessed_at
                   FROM files f LEFT JOIN file_contents c ON f.file_id = c.file_id
                  WHERE f.workspace_id = ?1 AND f.relative_path = ?2",
                rusqlite::params![&ws.0, path.as_str()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<Vec<u8>>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                    ))
                },
            );
            match row {
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
                Ok((file_id, is_cached, hash, blob, accessed)) => {
                    if is_cached == 0 {
                        return Ok(None);
                    }
                    let (Some(hash), Some(blob)) = (hash, blob) else {
                        // `is_cached` said yes and the blob is absent. Invariant 3 forbids this,
                        // and a schema test asserts it; treating it as a miss here means a
                        // divergence costs a refetch rather than a panic.
                        return Ok(None);
                    };
                    let Some(hash) = Sha256::parse(&hash) else {
                        return Ok(None);
                    };
                    let bytes = zstd::decode_all(&blob[..]).unwrap_or_default();
                    Ok(Some(CacheEntry {
                        file_id: FileId(file_id),
                        path: path.clone(),
                        hash,
                        bytes,
                        last_accessed_at: accessed.unwrap_or(0),
                    }))
                }
            }
        })
    }

    fn file_id(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<Option<FileId>> {
        self.with(|c| {
            let r = c.query_row(
                "SELECT file_id FROM files WHERE workspace_id = ?1 AND relative_path = ?2",
                rusqlite::params![&ws.0, path.as_str()],
                |r| r.get::<_, String>(0),
            );
            match r {
                Ok(id) => Ok(Some(FileId(id))),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    fn put_content(&self, file_id: &FileId, bytes: &[u8], hash: &Sha256, now: i64) -> StoreOutcome {
        if bytes.len() as u64 > CACHE_MAX_BYTES {
            return StoreOutcome::NotEligible {
                size: bytes.len() as u64,
            };
        }
        // Hash first, compress second (C1, §5.6): the stored digest is over decompressed bytes,
        // so it compares directly with the engine's. The caller supplies the hash it already
        // computed; this asserts the order rather than trusting it.
        debug_assert_eq!(
            hash,
            &Sha256::of(bytes),
            "the digest must be of the decompressed bytes"
        );
        let blob = match zstd::encode_all(bytes, ZSTD_LEVEL) {
            Ok(b) => b,
            Err(e) => return StoreOutcome::Failed(CacheError::Store(e.to_string())),
        };
        let r = self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO file_contents (file_id, content_blob, sha256_hash) VALUES (?1,?2,?3)
                 ON CONFLICT(file_id) DO UPDATE SET content_blob=?2, sha256_hash=?3",
                rusqlite::params![&file_id.0, &blob, hash.as_str()],
            )?;
            // `is_cached` and `last_accessed_at` are set in the same transaction as the blob they
            // describe (invariants 3 and 9), so the listing column can never disagree with what
            // opening the file would find.
            tx.execute(
                "UPDATE files SET is_cached=1, last_cached_at=?2, last_accessed_at=?2
                  WHERE file_id=?1",
                rusqlite::params![&file_id.0, now],
            )?;
            tx.commit()?;
            Ok(())
        });
        match r {
            Ok(()) => StoreOutcome::Stored,
            Err(e) => StoreOutcome::Failed(e),
        }
    }

    fn touch(&self, file_id: &FileId, now: i64) -> StoreOutcome {
        match self.with(|c| {
            c.execute(
                "UPDATE files SET last_accessed_at=?2 WHERE file_id=?1",
                rusqlite::params![&file_id.0, now],
            )?;
            Ok(())
        }) {
            Ok(()) => StoreOutcome::Stored,
            Err(e) => StoreOutcome::Failed(e),
        }
    }

    fn rename(&self, file_id: &FileId, to: &RelPath) -> CacheResult<()> {
        // Path columns only. `file_contents` is untouched, so the content survives the move —
        // which is what A-B5's opaque `file_id` was bought for (FR-022).
        let parent = to.parent().unwrap_or_else(RelPath::root);
        self.with(|c| {
            c.execute(
                "UPDATE files SET relative_path=?2, parent_path=?3, name=?4 WHERE file_id=?1",
                rusqlite::params![&file_id.0, to.as_str(), parent.as_str(), to.name()],
            )?;
            Ok(())
        })
    }

    fn mark_stale(&self, ws: &WorkspaceId, region: &RelPath) -> CacheResult<()> {
        // The whole workspace when the region is the root, which is what a wholesale
        // invalidation and a reconnection both produce. One statement either way.
        self.with(|c| {
            if region.as_str() == "/" || region.as_str().is_empty() {
                c.execute(
                    "UPDATE files SET stale = 1 WHERE workspace_id = ?1",
                    rusqlite::params![&ws.0],
                )?;
            } else {
                let prefix = format!("{}/", region.as_str());
                c.execute(
                    "UPDATE files SET stale = 1
                     WHERE workspace_id = ?1
                       AND (relative_path = ?2 OR (relative_path > ?3 AND relative_path < ?4))",
                    rusqlite::params![&ws.0, region.as_str(), &prefix, &upper_bound(&prefix)],
                )?;
            }
            Ok(())
        })
    }

    fn mark_unproven(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE file_contents SET unproven = 1
                 WHERE file_id IN (SELECT file_id FROM files
                                   WHERE workspace_id = ?1 AND relative_path = ?2)",
                rusqlite::params![&ws.0, path.as_str()],
            )?;
            Ok(())
        })
    }

    fn clear_unproven(&self, file_id: &FileId) -> CacheResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE file_contents SET unproven = 0 WHERE file_id = ?1",
                rusqlite::params![&file_id.0],
            )?;
            Ok(())
        })
    }

    fn rename_subtree(&self, ws: &WorkspaceId, from: &RelPath, to: &RelPath) -> CacheResult<usize> {
        let from_prefix = format!("{}/", from.as_str());
        let to_parent = to.parent().unwrap_or_else(RelPath::root);
        let cut = from.as_str().len();

        self.with(|c| {
            let tx = c.unchecked_transaction()?;

            // The directory's own row. Its `parent_path` has no `from` prefix to rewrite --
            // it is wherever the directory sat -- so the substring arithmetic that works for
            // every descendant would set the directory's parent to itself. It needs the
            // caller-derived destination parent instead, and that is why this is two
            // statements rather than one.
            let own = tx.execute(
                "UPDATE files SET relative_path = ?3, parent_path = ?4, name = ?5
                 WHERE workspace_id = ?1 AND relative_path = ?2",
                rusqlite::params![
                    &ws.0,
                    from.as_str(),
                    to.as_str(),
                    to_parent.as_str(),
                    to.name()
                ],
            )?;

            // Descendants. Bounded by a range comparison rather than `LIKE 'from/%'`: SQLite's
            // LIKE optimisation needs `case_sensitive_like` on with a BINARY column, which
            // `apply_pragmas` does not set, so LIKE would scan. The rule is identical; only
            // the plan differs.
            //
            // The trailing separator is the whole safety property. Matching on `from` alone
            // would rewrite `src-generated` when renaming `src`, silently corrupting rows
            // nobody touched.
            // One formula for both columns, and it took a failing test to see why. The
            // separator must stay in the remainder rather than being skipped: a descendant's
            // `parent_path` can be exactly `from`, where skipping it yields the empty string
            // and the new parent becomes `/syntax/` with a trailing separator -- which no
            // child ever matches, so the whole subtree becomes unreachable while every row
            // still looks right.
            let descendants = tx.execute(
                "UPDATE files
                    SET relative_path = ?4 || substr(relative_path, ?5),
                        parent_path   = ?4 || substr(parent_path, ?5)
                  WHERE workspace_id = ?1
                    AND relative_path > ?2 AND relative_path < ?3",
                rusqlite::params![
                    &ws.0,
                    &from_prefix,
                    &upper_bound(&from_prefix),
                    to.as_str(),
                    (cut + 1) as i64, // SQLite substr is 1-based; the separator stays
                ],
            )?;

            tx.commit()?;
            Ok(own + descendants)
        })
    }

    fn search_paths(
        &self,
        ws: &WorkspaceId,
        fragment: &str,
        limit: u32,
    ) -> CacheResult<Vec<RelPath>> {
        // `files_fts`, never a leading-wildcard LIKE (§5.2, §5.4). Never consults a provider (C6).
        let query = fts_prefix_query(fragment);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        self.with(|c| {
            // **`CROSS JOIN` pins the join order, and that is the whole performance of this
            // query.** With a plain `JOIN`, SQLite drives on `files` -- `workspace_id = ?2` looks
            // selective, so it takes `idx_files_lookup` as the outer loop and re-runs the MATCH
            // once per row. Measured on 50,000 cached paths: the MATCH alone answers in 1 ms and
            // the plain join took 12,345 ms, which is fifty thousand matches. `CROSS JOIN` is
            // SQLite's documented way to say "this table is the outer one" and costs nothing else.
            //
            // The `LIMIT` must stay **after** the workspace filter, which is why this is a join
            // order hint rather than a subquery limiting the MATCH: a match in another workspace
            // would otherwise consume the limit and hide this workspace's own results.
            let mut stmt = c.prepare(
                "SELECT f.relative_path
                   FROM files_fts CROSS JOIN files f ON f.rowid = files_fts.rowid
                  WHERE files_fts MATCH ?1 AND f.workspace_id = ?2
                  LIMIT ?3",
            )?;
            let rows = stmt.query_map(rusqlite::params![&query, &ws.0, limit], |r| {
                r.get::<_, String>(0)
            })?;
            let mut out = Vec::new();
            for row in rows {
                if let Ok(p) = RelPath::parse(&row?) {
                    out.push(p);
                }
            }
            Ok(out)
        })
    }

    fn evict(&self, before: i64) -> CacheResult<EvictionReport> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            let (blobs, bytes): (i64, i64) = tx.query_row(
                "SELECT count(*), coalesce(sum(length(c.content_blob)),0)
                   FROM file_contents c JOIN files f ON f.file_id = c.file_id
                  WHERE f.last_accessed_at IS NOT NULL AND f.last_accessed_at < ?1",
                [before],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            // Content only. **Never** a `files` row: the tree stays navigable and the file stays
            // listed (C4, FR-027, §5.5).
            tx.execute(
                "DELETE FROM file_contents WHERE file_id IN
                   (SELECT f.file_id FROM files f
                     WHERE f.last_accessed_at IS NOT NULL AND f.last_accessed_at < ?1)",
                [before],
            )?;
            tx.execute(
                "UPDATE files SET is_cached=0
                  WHERE last_accessed_at IS NOT NULL AND last_accessed_at < ?1",
                [before],
            )?;
            tx.commit()?;
            Ok(EvictionReport {
                blobs_removed: blobs as u64,
                bytes_reclaimed: bytes as u64,
            })
        })
    }

    fn workspace(&self, ws: &WorkspaceId) -> CacheResult<Option<Workspace>> {
        self.with(|c| {
            let found = c
                .query_row(
                    "SELECT name, location_type, base_path, last_opened_at
                       FROM workspaces WHERE workspace_id = ?1",
                    rusqlite::params![&ws.0],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    },
                )
                .ok();
            Ok(
                found.map(|(name, location_type, base, last_opened_at)| Workspace {
                    id: ws.clone(),
                    name,
                    location: if location_type == "LOCAL" {
                        Location::Local { base }
                    } else {
                        // The host is not stored: §5.2 keeps the base path and the kind, and the
                        // host belongs to the connection rather than to the workspace. Empty here
                        // rather than invented, and only the base path is ever read back.
                        Location::Remote {
                            host: String::new(),
                            base,
                        }
                    },
                    last_opened_at,
                }),
            )
        })
    }

    fn replace_git_status(
        &self,
        ws: &WorkspaceId,
        git: &crate::application::ports::workspace_cache::GitProjection,
    ) -> CacheResult<()> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            // Delete then insert, inside the transaction. A path that has stopped differing is
            // reported by its absence from the new answer, so anything less than a full replace
            // leaves it marked until the workspace is closed.
            tx.execute(
                "DELETE FROM git_status WHERE workspace_id = ?1",
                rusqlite::params![&ws.0],
            )?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO git_status (workspace_id, relative_path, status_type)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(workspace_id, relative_path) DO UPDATE SET status_type = ?3",
                )?;
                for change in &git.changes {
                    // `ON CONFLICT` rather than a plain insert, because git reports one state
                    // per path but the rows come off a wire: a duplicated path from a faulty
                    // engine must not fail the whole replacement (Principle VI).
                    stmt.execute(rusqlite::params![
                        &ws.0,
                        &change.path,
                        status_name(&change.status)
                    ])?;
                }
            }
            // The branch, in the same transaction as the changes it describes.
            let (kind, value) = branch_columns(&git.branch);
            tx.execute(
                "INSERT INTO git_branch (workspace_id, kind, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(workspace_id) DO UPDATE SET kind = ?2, value = ?3",
                rusqlite::params![&ws.0, kind, value],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    fn retain_edit(&self, ws: &WorkspaceId, path: &RelPath, edit: &PendingEdit) -> CacheResult<()> {
        let content = zstd::encode_all(&edit.content[..], ZSTD_LEVEL)
            .map_err(|e| CacheError::Store(e.to_string()))?;
        let base = match &edit.base {
            Some((bytes, hash)) => {
                let blob = zstd::encode_all(&bytes[..], ZSTD_LEVEL)
                    .map_err(|e| CacheError::Store(e.to_string()))?;
                Some((blob, hash.as_str().to_string()))
            }
            None => None,
        };
        let (base_blob, base_hash) = match base {
            Some((b, h)) => (Some(b), Some(h)),
            None => (None, None),
        };
        self.with(|c| {
            // `base_blob` and `base_sha256` are **not** in the UPDATE list. A second offline save
            // of one file replaces the content and leaves the base exactly where the first save
            // put it (FR-011b): re-deriving it from the new local content would make the merge
            // compare local against local and return a clean merge that is wrong. Expressing that
            // by omitting two columns is what makes it hold for every caller rather than for the
            // careful ones.
            c.execute(
                "INSERT INTO pending_edits
                     (workspace_id, relative_path, content_blob, base_blob, base_sha256,
                      mergeable, retained_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(workspace_id, relative_path) DO UPDATE SET
                     content_blob = ?3, mergeable = ?6, retained_at = ?7",
                rusqlite::params![
                    &ws.0,
                    path.as_str(),
                    &content,
                    &base_blob,
                    &base_hash,
                    edit.mergeable as i64,
                    edit.retained_at,
                ],
            )?;
            Ok(())
        })
    }

    fn pending_edits(&self, ws: &WorkspaceId) -> CacheResult<Vec<(RelPath, PendingEdit)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT relative_path, content_blob, base_blob, base_sha256, mergeable, retained_at
                   FROM pending_edits WHERE workspace_id = ?1 ORDER BY retained_at, relative_path",
            )?;
            let rows = stmt.query_map(rusqlite::params![&ws.0], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Option<Vec<u8>>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (raw, content, base_blob, base_hash, mergeable, retained_at) = row?;
                // Re-validated on read as well as on write (Principle VI). A row whose path does
                // not validate is **dropped, not repaired**: repairing it would invent a
                // destination for the developer's work, and the honest answer to a corrupt row is
                // that this path has no pending work.
                let Ok(path) = RelPath::parse(&raw) else {
                    continue;
                };
                let content = zstd::decode_all(&content[..]).unwrap_or_default();
                // Either both base columns are present or the file is treated as unmergeable. One
                // without the other is malformed, and merging against half a base is how a wrong
                // clean merge happens; prompting is the safe answer and the specified one.
                let (base, mergeable) = match (base_blob, base_hash) {
                    (Some(blob), Some(hex)) => {
                        match (zstd::decode_all(&blob[..]), Sha256::parse(&hex)) {
                            (Ok(bytes), Some(hash)) => (Some((bytes, hash)), mergeable != 0),
                            _ => (None, false),
                        }
                    }
                    (None, None) => (None, mergeable != 0),
                    _ => (None, false),
                };
                out.push((
                    path,
                    PendingEdit {
                        content,
                        base,
                        mergeable,
                        retained_at,
                    },
                ));
            }
            Ok(out)
        })
    }

    fn forget_pending(&self, ws: &WorkspaceId, path: &RelPath) -> CacheResult<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM pending_edits WHERE workspace_id = ?1 AND relative_path = ?2",
                rusqlite::params![&ws.0, path.as_str()],
            )?;
            Ok(())
        })
    }

    fn git_status(
        &self,
        ws: &WorkspaceId,
    ) -> CacheResult<crate::application::ports::workspace_cache::GitProjection> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT relative_path, status_type FROM git_status
                  WHERE workspace_id = ?1 ORDER BY relative_path",
            )?;
            let rows = stmt.query_map(rusqlite::params![&ws.0], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut changes = Vec::new();
            for row in rows {
                let (path, status) = row?;
                // A row whose status this build does not know is dropped, not guessed at. A
                // guess would mark a file with a state git never reported.
                if let Some(status) = status_from(&status) {
                    changes.push(apex_protocol::wire::GitChange { path, status });
                }
            }
            let stored = c
                .query_row(
                    "SELECT kind, value FROM git_branch WHERE workspace_id = ?1",
                    rusqlite::params![&ws.0],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .ok();
            // A workspace with no row has no branch, which is the same answer as a workspace
            // that is not a repository. The two are indistinguishable here on purpose (FR-027).
            let branch = stored
                .and_then(|(kind, value)| branch_from(&kind, value))
                .unwrap_or_default();
            Ok(crate::application::ports::workspace_cache::GitProjection { branch, changes })
        })
    }

    fn schema_version(&self) -> CacheResult<u32> {
        self.with(migrate::read_version)
    }

    fn migrate_to(&self, target: u32, progress: PhaseSink<'_>) -> Result<(), MigrationFailure> {
        let mut guard = self.conn.lock().map_err(|_| MigrationFailure::Step {
            version: 0,
            why: "connection poisoned".into(),
        })?;
        migrate::migrate(&mut guard, target, progress)
    }
}

/// Turn a user's fragment into an FTS5 prefix query, quoting it so punctuation in a path cannot
/// be read as query syntax.
///
/// A path fragment like `src/main.` is full of characters FTS5 treats as operators. Quoting makes
/// the fragment a literal, and the trailing `*` is what makes it a prefix search rather than an
/// exact-token match — which is what "find a file as I type" means.
fn fts_prefix_query(fragment: &str) -> String {
    let cleaned: String = fragment
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .map(|t| format!("\"{t}\"*"))
        .collect();
    tokens.join(" ")
}

/// The exclusive upper bound of a prefix range: `"src/"` becomes `"src0"`.
///
/// A range comparison rather than `LIKE` so the index is used, and bounded to the separator so
/// that renaming `src` cannot reach `src-generated`.
fn upper_bound(prefix: &str) -> String {
    let mut bound = prefix.to_string();
    let last = bound.pop().expect("a prefix is never empty");
    bound.push((last as u8 + 1) as char);
    bound
}

/// The stored spelling of a status. The wire's own spelling, deliberately: a second vocabulary
/// between the table and the protocol would be one more thing to keep in step, for no gain.
fn status_name(status: &apex_protocol::wire::GitStatusKind) -> &'static str {
    use apex_protocol::wire::GitStatusKind::*;
    match status {
        Modified => "MODIFIED",
        Untracked => "UNTRACKED",
        Staged => "STAGED",
        Deleted => "DELETED",
        Conflict => "CONFLICT",
    }
}

fn status_from(name: &str) -> Option<apex_protocol::wire::GitStatusKind> {
    use apex_protocol::wire::GitStatusKind::*;
    Some(match name {
        "MODIFIED" => Modified,
        "UNTRACKED" => Untracked,
        "STAGED" => Staged,
        "DELETED" => Deleted,
        "CONFLICT" => Conflict,
        _ => return None,
    })
}

fn branch_columns(branch: &apex_protocol::wire::BranchPosition) -> (&'static str, Option<&str>) {
    use apex_protocol::wire::BranchPosition::*;
    match branch {
        Branch(name) => ("branch", Some(name.as_str())),
        Detached(commit) => ("detached", Some(commit.as_str())),
        None => ("none", Option::None),
    }
}

fn branch_from(kind: &str, value: Option<String>) -> Option<apex_protocol::wire::BranchPosition> {
    use apex_protocol::wire::BranchPosition;
    Some(match (kind, value) {
        ("branch", Some(v)) => BranchPosition::Branch(v),
        ("detached", Some(v)) => BranchPosition::Detached(v),
        ("none", _) => BranchPosition::None,
        // A named case with no name is a row that cannot mean anything. Reported as no branch
        // rather than as an empty one, which would render as a blank label (FR-019).
        _ => return Option::None,
    })
}
