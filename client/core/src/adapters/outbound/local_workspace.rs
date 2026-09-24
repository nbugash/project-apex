//! Outbound adapter: `WorkspaceProvider` over the local filesystem (§6.4).
//!
//! Used by Local Mode (§13) and by cloud burst's pre-upload view (§15.4). The UI never learns it
//! is here rather than across a link — which is the property FR-001 exists for, and the reason
//! the same contract suite runs against this and the remote adapter alike.
//!
//! **Subject to the same containment rule.** §6.4 says it outright: a local workspace is still a
//! workspace, and path escapes are still bugs. The check is implemented here rather than shared
//! with the engine's because they are genuinely two enforcements — Principle VI requires each
//! side to validate independently, and a shared implementation would make "independently" a
//! word rather than a fact. The *algorithm* is deliberately the same two stages, and the reason
//! for the order is the same: `canonicalize` fails on a path that does not exist, so a
//! single-stage check would answer "no such file" for an escape to a missing target and
//! "refused" for an escape to a real one, telling a caller what lives outside the workspace.

use crate::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WorkspaceProvider,
};
use crate::domain::workspace::{
    ByteRange, DirPage, EntryKind, FileChunk, FsEntry, FsMeta, PageRequest, RelPath, Sha256,
    WorkspaceId,
};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct LocalWorkspaceProvider {
    /// Canonicalised once, at construction. Every later request is then a resolve and a prefix
    /// comparison rather than a second canonicalisation of the root.
    root: PathBuf,
}

impl LocalWorkspaceProvider {
    /// Fails when the root is not a readable directory — refused here rather than on the first
    /// read, so the failure names the workspace instead of a file inside it.
    pub fn open(root: &Path) -> std::io::Result<Self> {
        let root = std::fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a workspace root must be a directory",
            ));
        }
        Ok(Self { root })
    }

    /// Resolve a workspace-relative path, refusing anything that leaves the root.
    ///
    /// `RelPath` has already rejected `..`, absolute forms and NUL on construction. That is the
    /// lexical stage and it is not enough on its own: a path with no `..` in it can still leave
    /// the root through a symlink, and only resolving it catches that (FR-006).
    async fn resolve(&self, path: &RelPath) -> ProviderResult<PathBuf> {
        let mut joined = self.root.clone();
        for part in path.as_str().split('/').filter(|p| !p.is_empty()) {
            joined.push(part);
        }

        match tokio::fs::canonicalize(&joined).await {
            Ok(real) if real.starts_with(&self.root) => Ok(real),
            // A symlink took it out of the root.
            Ok(_) => Err(ProviderError::Refused),
            Err(_) => {
                // The leaf is not there. Whether that is an honest miss or an escape depends on
                // where its parent lands, so resolve the parent: a contained parent means a
                // miss, and a parent outside the root means the caller was reaching out and
                // must not learn whether the target exists (FR-007).
                let parent = joined.parent().unwrap_or(&joined).to_path_buf();
                match tokio::fs::canonicalize(&parent).await {
                    Ok(p) if p.starts_with(&self.root) => Err(ProviderError::NotFound),
                    _ => Err(ProviderError::Refused),
                }
            }
        }
    }
}

fn secs(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl WorkspaceProvider for LocalWorkspaceProvider {
    async fn read_directory(
        &self,
        _ws: &WorkspaceId,
        path: &RelPath,
        page: PageRequest,
    ) -> ProviderResult<DirPage> {
        let dir = self.resolve(path).await?;
        let mut reader = tokio::fs::read_dir(&dir)
            .await
            .map_err(|_| ProviderError::NotFound)?;

        let mut items: Vec<FsEntry> = Vec::new();
        while let Ok(Some(entry)) = reader.next_entry().await {
            // `metadata()` follows symlinks, so a symlinked directory inside the workspace shows
            // as a directory. Containment is enforced when a path is *resolved*, not when it is
            // listed — listing a name reveals nothing that reading it would not.
            let Ok(m) = entry.metadata().await else {
                continue;
            };
            let Ok(name) = entry.file_name().into_string() else {
                continue; // not UTF-8, so it cannot travel the same shape as a remote listing
            };
            items.push(FsEntry {
                name,
                kind: if m.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: m.len(),
                modified: secs(&m),
            });
        }

        // The same contractual order the wire promises, so a consumer cannot tell the two
        // providers apart by the shape of what comes back.
        items.sort_by(FsEntry::listing_order);

        if let Some(after) = &page.cursor {
            let at = items
                .iter()
                .position(|e| cursor_token(e).as_str() > after.as_str())
                .unwrap_or(items.len());
            items.drain(..at);
        }
        let more = items.len() > page.limit as usize;
        items.truncate(page.limit as usize);
        let next_cursor = more.then(|| items.last().map(cursor_token)).flatten();
        Ok(DirPage { items, next_cursor })
    }

    async fn stat(&self, _ws: &WorkspaceId, path: &RelPath) -> ProviderResult<FsMeta> {
        let target = self.resolve(path).await?;
        let m = tokio::fs::metadata(&target)
            .await
            .map_err(|_| ProviderError::NotFound)?;
        let sha256 = if m.is_dir() {
            None
        } else {
            let bytes = tokio::fs::read(&target)
                .await
                .map_err(|_| ProviderError::NotFound)?;
            Some(Sha256::of(&bytes))
        };
        Ok(FsMeta {
            kind: if m.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: m.len(),
            modified: secs(&m),
            sha256,
        })
    }

    async fn read_file(
        &self,
        _ws: &WorkspaceId,
        path: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        let target = self.resolve(path).await?;
        // Read whole, then slice. There is no transport to protect here, so the inline limit
        // that governs the remote adapter does not apply — but the *shape* must match, because
        // the contract suite asserts both against the same guarantees.
        let whole = tokio::fs::read(&target)
            .await
            .map_err(|_| ProviderError::NotFound)?;
        let total = whole.len() as u64;
        let r = range.unwrap_or(ByteRange {
            offset: 0,
            length: total,
        });
        let start = (r.offset as usize).min(whole.len());
        let end = start.saturating_add(r.length as usize).min(whole.len());
        Ok(FileChunk {
            range: ByteRange {
                offset: r.offset,
                length: (end - start) as u64,
            },
            bytes: whole[start..end].to_vec(),
            total_size: total,
            // The whole file's digest, never the range's (FR-021).
            sha256: Sha256::of(&whole),
        })
    }
}

/// The same opaque pagination token the engine produces.
///
/// Encoding only the name is wrong the moment a directory and a file interleave: with
/// directories `a` and `z` and a file `b` the listing is `a, z, b`, and a name-only cursor
/// resuming after `z` finds no later name and drops `b`.
fn cursor_token(e: &FsEntry) -> String {
    let group = if matches!(e.kind, EntryKind::Directory) {
        '0'
    } else {
        '1'
    };
    format!("{group}\u{1f}{}", e.name)
}
