//! An in-memory `WorkspaceProvider`.
//!
//! Not a stub. It reproduces the behaviours `CachedWorkspace`'s rules depend on and, crucially,
//! **fails on demand**: a "never answers" switch is what makes the confirmation limit (FR-021c,
//! SC-004b) testable at all, since a wedged engine is not otherwise producible.

#![allow(dead_code)]

use apex_shell::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WorkspaceProvider,
};
use apex_shell::domain::workspace::{
    ByteRange, DirPage, EntryKind, FileChunk, FsEntry, FsMeta, PageRequest, RelPath, Sha256,
    WorkspaceId,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeWorkspace {
    /// Path -> contents. A directory is an entry with `None`.
    nodes: Mutex<BTreeMap<String, Option<Vec<u8>>>>,
    /// Every provider call, in order. Counted here rather than parsed out of a log: a log-shape
    /// change would silently pass a test that greps.
    pub calls: Mutex<Vec<String>>,
    /// Requests that must never be answered, by method name.
    wedged: Mutex<Vec<String>>,
    /// Bytes of content handed out, so "transfers zero bytes" is a measurement.
    pub content_bytes: AtomicUsize,
    offline: Mutex<bool>,
    gone: Mutex<bool>,
}

impl FakeWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file(&self, path: &str, bytes: &[u8]) -> &Self {
        let mut here = String::new();
        let mut parts: Vec<&str> = path.split('/').collect();
        parts.pop();
        for p in parts {
            if p.is_empty() {
                continue;
            }
            here.push('/');
            here.push_str(p);
            self.nodes
                .lock()
                .unwrap()
                .entry(here.clone())
                .or_insert(None);
        }
        self.nodes
            .lock()
            .unwrap()
            .insert(path.to_string(), Some(bytes.to_vec()));
        self
    }

    pub fn dir(&self, path: &str) -> &Self {
        self.nodes.lock().unwrap().insert(path.to_string(), None);
        self
    }

    /// Change content underneath a cached copy, which is how a stale hash is produced.
    pub fn rewrite(&self, path: &str, bytes: &[u8]) {
        self.nodes
            .lock()
            .unwrap()
            .insert(path.to_string(), Some(bytes.to_vec()));
    }

    pub fn remove(&self, path: &str) {
        self.nodes.lock().unwrap().remove(path);
    }

    /// Never answer this method. The engine reachable but wedged (FR-021c).
    pub fn wedge(&self, method: &str) {
        self.wedged.lock().unwrap().push(method.to_string());
    }

    pub fn set_offline(&self, v: bool) {
        *self.offline.lock().unwrap() = v;
    }

    pub fn set_gone(&self, v: bool) {
        *self.gone.lock().unwrap() = v;
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == method)
            .count()
    }

    pub fn total_calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn bytes_transferred(&self) -> usize {
        self.content_bytes.load(Ordering::SeqCst)
    }

    fn record(&self, method: &str) -> ProviderResult<()> {
        self.calls.lock().unwrap().push(method.to_string());
        if *self.gone.lock().unwrap() {
            return Err(ProviderError::WorkspaceGone);
        }
        if *self.offline.lock().unwrap() {
            return Err(ProviderError::Offline);
        }
        Ok(())
    }

    /// A wedged method never returns. `pending()` is an honest model of it: the future simply
    /// never completes, which is what a caller's timeout has to cope with.
    async fn maybe_wedge(&self, method: &str) {
        if self.wedged.lock().unwrap().iter().any(|m| m == method) {
            std::future::pending::<()>().await;
        }
    }
}

#[async_trait]
impl WorkspaceProvider for FakeWorkspace {
    async fn read_directory(
        &self,
        _ws: &WorkspaceId,
        path: &RelPath,
        page: PageRequest,
    ) -> ProviderResult<DirPage> {
        self.record("read_directory")?;
        self.maybe_wedge("read_directory").await;
        let base = path.as_str().to_string();
        let prefix = if base == "/" {
            "/".to_string()
        } else {
            format!("{base}/")
        };
        let nodes = self.nodes.lock().unwrap();
        let mut items: Vec<FsEntry> = nodes
            .iter()
            .filter_map(|(p, c)| {
                let rest = p.strip_prefix(&prefix)?;
                if rest.is_empty() || rest.contains('/') {
                    return None;
                }
                Some(FsEntry {
                    name: rest.to_string(),
                    kind: if c.is_none() {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    size: c.as_ref().map(|b| b.len() as u64).unwrap_or(0),
                    modified: 0,
                })
            })
            .collect();
        items.sort_by(FsEntry::listing_order);
        if let Some(after) = &page.cursor {
            let at = items
                .iter()
                .position(|e| &e.name == after)
                .map(|i| i + 1)
                .unwrap_or(0);
            items.drain(..at);
        }
        let more = items.len() > page.limit as usize;
        items.truncate(page.limit as usize);
        let next_cursor = more.then(|| items.last().map(|e| e.name.clone())).flatten();
        Ok(DirPage { items, next_cursor })
    }

    async fn stat(&self, _ws: &WorkspaceId, path: &RelPath) -> ProviderResult<FsMeta> {
        self.record("stat")?;
        self.maybe_wedge("stat").await;
        let nodes = self.nodes.lock().unwrap();
        match nodes.get(path.as_str()) {
            Some(Some(c)) => Ok(FsMeta {
                kind: EntryKind::File,
                size: c.len() as u64,
                modified: 0,
                sha256: Some(Sha256::of(c)),
            }),
            Some(None) => Ok(FsMeta {
                kind: EntryKind::Directory,
                size: 0,
                modified: 0,
                sha256: None,
            }),
            None => Err(ProviderError::NotFound),
        }
    }

    async fn read_file(
        &self,
        _ws: &WorkspaceId,
        path: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        self.record("read_file")?;
        self.maybe_wedge("read_file").await;
        let nodes = self.nodes.lock().unwrap();
        let Some(Some(content)) = nodes.get(path.as_str()) else {
            return Err(ProviderError::NotFound);
        };
        let total = content.len() as u64;
        let r = range.unwrap_or(ByteRange {
            offset: 0,
            length: total,
        });
        let start = (r.offset as usize).min(content.len());
        let end = start.saturating_add(r.length as usize).min(content.len());
        let bytes = content[start..end].to_vec();
        self.content_bytes.fetch_add(bytes.len(), Ordering::SeqCst);
        Ok(FileChunk {
            range: ByteRange {
                offset: r.offset,
                length: bytes.len() as u64,
            },
            bytes,
            total_size: total,
            sha256: Sha256::of(content),
        })
    }
}
