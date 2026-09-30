//! Hold a save the host has not seen.
//!
//! **Takes only the cache.** There is no provider here to fall back to, which makes "an offline
//! save contacts nothing" structural rather than a promise -- the same reason `SearchPaths` holds
//! only the cache, and the same reason `TextMerge` is a pure port.
//!
//! The use case is thin on purpose: the one decision it makes is the one that matters, which is
//! what happens to the base when a path already carries work.

use crate::application::ports::workspace_cache::{CacheResult, PendingEdit, WorkspaceCache};
use crate::domain::workspace::{RelPath, Sha256, WorkspaceId};
use std::sync::Arc;

pub struct RetainEdit {
    cache: Arc<dyn WorkspaceCache>,
}

impl RetainEdit {
    pub fn new(cache: Arc<dyn WorkspaceCache>) -> Self {
        Self { cache }
    }

    /// Retain what the developer saved, against the content the host last confirmed.
    ///
    /// `base` is `None` for a file created offline: there is nothing for it to differ from, and a
    /// base of `""` would read as real content everywhere downstream.
    ///
    /// **On a path that already carries work, the content advances and the base does not**
    /// (FR-011b, FR-011c). The store enforces that by omitting two columns from its UPDATE, so
    /// this method passes the base it was given and lets the store ignore it for a replacement.
    /// Stating it in both places is deliberate: re-deriving the base from newer local content makes
    /// the eventual merge compare local against local and return a clean merge that is *wrong*,
    /// and no assertion about success would notice.
    ///
    /// `mergeable` is decided by the caller, from what the client already holds -- the editor either
    /// decoded this file as text within its limit or it did not (research.md). No content sniffing
    /// here: a second opinion about one file is a second opinion that eventually disagrees.
    ///
    /// Errors are **returned**, never absorbed. FR-016 requires the developer be told while the
    /// work is still in the buffer, and a use case that logged and returned `Ok` would leave them
    /// believing it was held.
    pub fn save(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        content: &[u8],
        base: Option<(&[u8], &Sha256)>,
        mergeable: bool,
        now: i64,
    ) -> CacheResult<()> {
        let edit = PendingEdit {
            content: content.to_vec(),
            base: base.map(|(bytes, hash)| (bytes.to_vec(), hash.clone())),
            mergeable,
            retained_at: now,
        };
        self.cache.retain_edit(ws, path, &edit)
    }
}
