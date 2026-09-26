//! Where every rule about serving cached content lives.
//!
//! This implements `WorkspaceProvider` **and** consumes one, which looks like a layering smell and
//! is not. Everything it does is a rule — a hash match serves from cache, a mismatch fetches,
//! nothing is shown unverified while connected, a disconnection changes what may be shown, every
//! hit records an access, a caching failure never fails the read. Principle VIII puts rules in the
//! application layer and keeps them out of adapters, and the practical consequence is the point:
//! all of it is exercisable against an in-memory fake cache, a fake engine and a fake clock, with
//! no SQLite file, no process and no network.

use crate::application::ports::clock::Clock;
use crate::application::ports::connection::ConnectionStatusSource;
use crate::application::ports::workspace_cache::{StoreOutcome, WorkspaceCache};
use crate::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WorkspaceProvider,
};
use crate::domain::cache::{Presentation, Validity};
use crate::domain::connection::ConnectionState;
use crate::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsMeta, PageRequest, RelPath, Sha256, WorkspaceId,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// A-DEADLINE's first instance: the confirmation limit.
pub const CONFIRM_LIMIT: Duration = Duration::from_secs(2);

/// Where a `Presentation` is published.
pub type PresentationSink = Arc<dyn Fn(Presentation) + Send + Sync>;

/// The numbers this use case is built against, injected so tests need not wait for them.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub confirm: Duration,
    pub bulk_threshold: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            confirm: CONFIRM_LIMIT,
            bulk_threshold: apex_protocol::wire::MAX_INLINE_READ,
        }
    }
}

pub struct CachedWorkspace {
    inner: Arc<dyn WorkspaceProvider>,
    cache: Arc<dyn WorkspaceCache>,
    clock: Arc<dyn Clock>,
    connection: Arc<dyn ConnectionStatusSource>,
    present: PresentationSink,
    limits: Limits,
}

impl CachedWorkspace {
    pub fn new(
        inner: Arc<dyn WorkspaceProvider>,
        cache: Arc<dyn WorkspaceCache>,
        clock: Arc<dyn Clock>,
        connection: Arc<dyn ConnectionStatusSource>,
        present: PresentationSink,
        limits: Limits,
    ) -> Self {
        Self {
            inner,
            cache,
            clock,
            connection,
            present,
            limits,
        }
    }

    /// Consulted **before** the cache, never after a failure.
    ///
    /// An outage must cost nothing and produce no timeout. Discovering it by letting a request
    /// fail would make every offline open wait for the transport to give up first.
    fn connected(&self) -> bool {
        matches!(self.connection.current(), ConnectionState::Connected)
    }

    fn publish(&self, p: Presentation) {
        (self.present)(p);
    }
}

#[async_trait]
impl WorkspaceProvider for CachedWorkspace {
    /// Consult the projection first; on a miss issue **one** shallow request (FR-014, FR-015,
    /// FR-016, §10.1).
    async fn read_directory(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        page: PageRequest,
    ) -> ProviderResult<DirPage> {
        if let Ok(cached) = self.cache.list_children(ws, path) {
            if !cached.is_empty() {
                // A folder already listed is never re-requested while it remains valid.
                return Ok(DirPage {
                    items: cached,
                    next_cursor: None,
                });
            }
        }
        if !self.connected() {
            // Offline and unfetched: say so rather than appearing empty.
            self.publish(Presentation::Unavailable);
            return Err(ProviderError::Offline);
        }
        let fetched = self.inner.read_directory(ws, path, page).await?;
        // Failing to persist a listing must not fail the listing.
        let _ = self.cache.put_listing(ws, path, &fetched.items);
        Ok(fetched)
    }

    async fn stat(&self, ws: &WorkspaceId, path: &RelPath) -> ProviderResult<FsMeta> {
        if !self.connected() {
            return Err(ProviderError::Offline);
        }
        self.inner.stat(ws, path).await
    }

    /// The flow FR-021a through FR-021c specify, in the order they specify it.
    async fn read_file(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        let cached = self.cache.lookup(ws, path).ok().flatten();

        if !self.connected() {
            return match cached {
                // Served, and presented as possibly stale rather than as current (FR-032).
                Some(entry) => {
                    self.publish(Presentation::PossiblyStale);
                    Ok(chunk_from(entry.bytes, range, entry.hash))
                }
                // Reported unavailable rather than shown as an empty document (FR-033).
                None => {
                    self.publish(Presentation::Unavailable);
                    Err(ProviderError::Offline)
                }
            };
        }

        let Some(entry) = cached else {
            return self.fetch_and_cache(ws, path, range).await;
        };

        // Published **before** the stat is issued, and for the whole time it is outstanding
        // (FR-021b). No bytes reach the caller until one of the three branches resolves
        // (FR-021a).
        self.publish(Presentation::Verifying);

        let confirmed = tokio::time::timeout(self.limits.confirm, self.inner.stat(ws, path)).await;

        match confirmed {
            Ok(Ok(meta)) => match Validity::compare(Some(&entry.hash), meta.sha256.as_ref()) {
                Validity::Valid => {
                    // Every hit records its access, so retention measures use rather than age
                    // (FR-028). A failure to record is not a failure to read.
                    let _ = self.cache.touch(&entry.file_id, self.clock.now());
                    // The hash agreed, so whatever doubt an event cast is now settled. Clearing
                    // it here rather than when the event arrives is the point of A-UNPROVEN:
                    // the existing comparison is the only thing that decides, and `unproven` is
                    // a hint that it will disagree rather than a second mechanism.
                    let _ = self.cache.clear_unproven(&entry.file_id);
                    self.publish(Presentation::Current);
                    Ok(chunk_from(entry.bytes, range, entry.hash))
                }
                Validity::Stale | Validity::Absent => self.fetch_and_cache(ws, path, range).await,
            },
            Ok(Err(ProviderError::WorkspaceGone)) => {
                // Not staleness: the thing being projected does not exist, so there is nothing
                // to qualify (FR-038).
                self.publish(Presentation::Gone);
                Err(ProviderError::WorkspaceGone)
            }
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => {
                // The engine is reachable but wedged. End the wait, say the content could not be
                // verified, and offer the cached copy marked unverified (FR-021c). An unbounded
                // wait is not an option: a wedged engine is exactly when the cache is most useful.
                self.publish(Presentation::Unverified);
                Ok(chunk_from(entry.bytes, range, entry.hash))
            }
        }
    }

    /// Save, then make the projection agree with what was saved (FR-010).
    ///
    /// Forwarded rather than inherited. The trait's default refuses with `Unsupported`, and a
    /// decorator that inherited it would refuse every save while the adapter underneath was
    /// perfectly able to perform one -- a capability lost in the wrapper, with the tests on
    /// either side of it passing. That is how four earlier features came to be marked complete
    /// without a request ever crossing the seam.
    async fn write_file(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        content: &[u8],
        base: &Sha256,
    ) -> ProviderResult<Sha256> {
        // The cache is touched only **after** the engine confirms, and never before. A refused
        // write that had already updated the projection would show the developer their own
        // rejected attempt the next time they opened the file offline, as though it had landed.
        let written = self.inner.write_file(ws, path, content, base).await?;

        if let Ok(Some(file_id)) = self.cache.file_id(ws, path) {
            // The engine's digest describes what reached the disk, which is not always what was
            // sent -- a mount that translates line endings is enough to part them. Storing our
            // bytes under the engine's digest would make the projection claim content the host
            // does not have, and the hash comparison guarding every read would agree with it.
            // So the two are compared: equal, cache; unequal, mark the entry unproven so the
            // next read fetches instead of trusting.
            if Sha256::of(content) == written {
                // Caching is an optimisation here for the same reason it is on the read path: a
                // full disk changes what is stored, never whether the save succeeded (FR-034).
                match self
                    .cache
                    .put_content(&file_id, content, &written, self.clock.now())
                {
                    StoreOutcome::Stored => {}
                    StoreOutcome::NotEligible { size } => {
                        tracing_note(&format!("not caching {path}: {size} bytes exceeds the cap"));
                    }
                    StoreOutcome::Failed(e) => {
                        tracing_note(&format!("failed to cache {path}: {e}"));
                    }
                }
            } else {
                let _ = self.cache.mark_unproven(ws, path);
            }
        }
        Ok(written)
    }
}

impl CachedWorkspace {
    async fn fetch_and_cache(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        let chunk = self.inner.read_file(ws, path, range).await?;

        // Caching is an optimisation. A full disk, a locked database or a file above the
        // eligibility cap changes what is stored, never what is returned (FR-034). The outcome is
        // deliberately inspected rather than discarded, so a store failure is visible in a log
        // without ever becoming the caller's problem.
        if range.is_none() {
            if let Ok(Some(file_id)) = self.cache.file_id(ws, path) {
                match self.cache.put_content(
                    &file_id,
                    &chunk.bytes,
                    &chunk.sha256,
                    self.clock.now(),
                ) {
                    StoreOutcome::Stored => {}
                    StoreOutcome::NotEligible { size } => {
                        tracing_note(&format!("not caching {path}: {size} bytes exceeds the cap"));
                    }
                    StoreOutcome::Failed(e) => {
                        tracing_note(&format!("failed to cache {path}: {e}"));
                    }
                }
            }
        }
        self.publish(Presentation::Current);
        Ok(chunk)
    }
}

/// Serve a range out of whole content already held.
fn chunk_from(
    bytes: Vec<u8>,
    range: Option<ByteRange>,
    hash: crate::domain::workspace::Sha256,
) -> FileChunk {
    let total = bytes.len() as u64;
    match range {
        None => FileChunk {
            range: ByteRange {
                offset: 0,
                length: total,
            },
            bytes,
            total_size: total,
            sha256: hash,
        },
        Some(r) => {
            let start = (r.offset as usize).min(bytes.len());
            let end = start.saturating_add(r.length as usize).min(bytes.len());
            let slice = bytes[start..end].to_vec();
            FileChunk {
                range: ByteRange {
                    offset: r.offset,
                    length: slice.len() as u64,
                },
                bytes: slice,
                total_size: total,
                // Still the whole file's digest, never the range's (FR-021).
                sha256: hash,
            }
        }
    }
}

/// Caching failures are logged, never propagated. Routed through one place so the rule is visible.
fn tracing_note(message: &str) {
    crate::logging::warn(message);
}
