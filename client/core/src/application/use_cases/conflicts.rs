//! The conflicts a reconciliation left, listed with all three sides and resolved one file at a time.
//!
//! **Nothing here is stored.** The durable fact is the `pending_edits` row; which rows are conflicts
//! is the last reconciliation's statement, passed in; and the remote side is read on every listing,
//! so it cannot go stale while the developer decides (`contracts/offline-commands.md`).
//!
//! **A resolution is conditional on the remote the developer was shown**, not on one read at
//! resolve time. That is the only way a host that moved during the conversation is noticed at all:
//! re-reading would make the write conditional on the newer remote and silently overwrite it.

use crate::application::ports::text_merge::{MergeOutcome, TextMerge};
use crate::application::ports::workspace_cache::WorkspaceCache;
use crate::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use crate::application::use_cases::reconcile::absent_file_digest;
use crate::domain::workspace::{RelPath, Sha256, WorkspaceId};
use std::sync::Arc;

/// Why the developer is being asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    /// Both sides changed the same region.
    Overlap,
    /// The client cannot merge the file as text, so it always asks (FR-025a).
    NotText,
    /// The host no longer has the file (FR-026).
    DeletedOnHost,
    /// Created offline, and the host has a file at the same path.
    CreatedOnBothSides,
}

/// One conflict, as the developer sees it.
///
/// A text is `None` where its side does not exist or is not UTF-8; the `_present` flags say which,
/// so an absent text is never ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub relative_path: RelPath,
    pub base: Option<String>,
    pub local: Option<String>,
    pub remote: Option<String>,
    /// False only for a file created offline (guarantee 3). Eviction does not clear it.
    pub base_present: bool,
    pub remote_present: bool,
    /// What `resolve` must be told back, so the write is conditional on what was shown.
    pub remote_sha256: Option<Sha256>,
    pub mergeable: bool,
    pub reason: ConflictReason,
    /// For an overlap: the combination with markers around the colliding regions only (US4
    /// scenario 7), for the developer to edit. Never written as it stands.
    pub draft: Option<String>,
}

/// What the developer chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Their edited result.
    Text(String),
    /// The offline bytes as they are. The only way to keep an unmergeable file's work.
    KeepLocal,
    /// The host's side, including its deletion. Nothing is written.
    TakeRemote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// Written (or, for `TakeRemote`, accepted) and the retained work forgotten.
    Resolved,
    /// The host moved while the developer decided. A new conflict, against the newer remote.
    Conflicted,
    /// Not attempted, and nothing changed: why, in words.
    Refused(String),
    /// The connection is gone. The work stays; ask again once it returns.
    NotAttempted,
    Failed(String),
}

pub struct Conflicts {
    cache: Arc<dyn WorkspaceCache>,
    provider: Arc<dyn WorkspaceProvider>,
    merge: Arc<dyn TextMerge>,
}

impl Conflicts {
    pub fn new(
        cache: Arc<dyn WorkspaceCache>,
        provider: Arc<dyn WorkspaceProvider>,
        merge: Arc<dyn TextMerge>,
    ) -> Self {
        Self {
            cache,
            provider,
            merge,
        }
    }

    /// The pending rows among `conflicted`, in the order they were retained, each with its remote
    /// read now. A path in `conflicted` with no row -- resolved since -- is simply absent.
    ///
    /// `Err` only for what makes the whole listing unanswerable: the store, or the connection. A
    /// deleted file is an answer (`remote_present: false`), not an error.
    pub async fn list(
        &self,
        ws: &WorkspaceId,
        conflicted: &[RelPath],
    ) -> Result<Vec<Conflict>, ProviderError> {
        let rows = self
            .cache
            .pending_edits(ws)
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let mut out = Vec::new();
        for (path, edit) in rows.into_iter().filter(|(p, _)| conflicted.contains(p)) {
            let remote = match self.provider.read_file(ws, &path, None).await {
                Ok(chunk) => Some(chunk),
                Err(ProviderError::NotFound) => None,
                Err(e) => return Err(e),
            };
            let text = |b: &[u8]| String::from_utf8(b.to_vec()).ok();
            let base = edit.base.as_ref().and_then(|(b, _)| text(b));
            let local = text(&edit.content);
            let remote_text = remote.as_ref().and_then(|c| text(&c.bytes));
            let all_text =
                local.is_some() && remote_text.is_some() && (edit.base.is_none() || base.is_some());
            let reason = if !edit.mergeable || (remote.is_some() && !all_text) {
                ConflictReason::NotText
            } else if remote.is_none() {
                ConflictReason::DeletedOnHost
            } else if edit.base.is_none() {
                ConflictReason::CreatedOnBothSides
            } else {
                ConflictReason::Overlap
            };
            let draft = match (reason, &base, &local, &remote_text) {
                (ConflictReason::Overlap, Some(b), Some(l), Some(r)) => {
                    Some(match self.merge.merge(b, l, r) {
                        MergeOutcome::Conflict(draft) | MergeOutcome::Clean(draft) => draft,
                    })
                }
                _ => None,
            };
            out.push(Conflict {
                relative_path: path,
                base_present: edit.base.is_some(),
                remote_present: remote.is_some(),
                remote_sha256: remote.as_ref().map(|c| c.sha256.clone()),
                mergeable: edit.mergeable && reason != ConflictReason::NotText,
                base,
                local,
                remote: remote_text,
                reason,
                draft,
            });
        }
        Ok(out)
    }

    /// Settle one file. Writes and forgets together, or neither (guarantee 1); touches no other
    /// row (guarantee 3); a stale refusal is `Conflicted` (guarantee 2).
    ///
    /// `seen` is the remote digest the developer was shown, `None` when they were shown a deletion.
    pub async fn resolve(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        resolution: Resolution,
        seen: Option<Sha256>,
    ) -> ResolveOutcome {
        let edit = match self.cache.pending_edits(ws) {
            Ok(rows) => rows.into_iter().find(|(p, _)| p == path).map(|(_, e)| e),
            Err(e) => return ResolveOutcome::Failed(e.to_string()),
        };
        let Some(edit) = edit else {
            return ResolveOutcome::Refused("there is no retained work for this file".into());
        };
        let content = match resolution {
            Resolution::TakeRemote => return self.forget(ws, path),
            Resolution::KeepLocal => edit.content,
            Resolution::Text(text) if carries_markers(&text) => {
                return ResolveOutcome::Refused(
                    "the result still has conflict markers; settle each marked region first".into(),
                )
            }
            Resolution::Text(text) => text.into_bytes(),
        };
        let base = seen.unwrap_or_else(absent_file_digest);
        match self.provider.write_file(ws, path, &content, &base).await {
            Ok(_) => self.forget(ws, path),
            Err(ProviderError::WriteConflict) => ResolveOutcome::Conflicted,
            Err(ProviderError::Offline) => ResolveOutcome::NotAttempted,
            Err(e) => ResolveOutcome::Failed(e.to_string()),
        }
    }

    fn forget(&self, ws: &WorkspaceId, path: &RelPath) -> ResolveOutcome {
        match self.cache.forget_pending(ws, path) {
            Ok(()) => ResolveOutcome::Resolved,
            Err(e) => ResolveOutcome::Failed(format!("the local row remains: {e}")),
        }
    }
}

/// Whether a text still has an unsettled region: an opening and a closing marker line.
///
/// Both, so a file that merely mentions one marker in prose is not refused; a real draft always
/// carries the pair.
fn carries_markers(text: &str) -> bool {
    text.lines().any(|l| l.starts_with("<<<<<<<")) && text.lines().any(|l| l.starts_with(">>>>>>>"))
}
