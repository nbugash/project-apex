//! Send the work the host has not seen, once the connection returns.
//!
//! **Every failure is an `Outcome`, never an `Err`.** A reconciliation that returned `Err` would
//! lose the per-file detail FR-024 requires: the developer is told what happened to each file, and
//! one error for the run says nothing about which files landed.
//!
//! **A row is deleted only where the host confirmed a write.** Every other path -- a conflict, a
//! refused stale write, a connection that dropped, a root that is gone -- leaves it alone. That is
//! what makes "nothing is lost" (FR-022) true by construction rather than by care, and it is why
//! `forget_pending` is called in exactly one place in this file.

use crate::application::ports::text_merge::{MergeOutcome, TextMerge};
use crate::application::ports::workspace_cache::WorkspaceCache;
use crate::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use crate::domain::connection::ConnectionState;
use crate::domain::workspace::{RelPath, Sha256, WorkspaceId};
use std::sync::Arc;

/// What happened to one file on one reconciliation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The host had not moved, so the local content was written as it stood.
    ///
    /// Distinct from `Merged` although both succeed and both delete the row. FR-024 requires the
    /// report say per file what happened, and reporting a merge for a fast-forward sends a
    /// developer to review a combination that never occurred.
    FastForwarded,
    /// The host had moved, the changes did not collide, and the combination was written.
    Merged,
    /// The developer decides. The row stays until they do (FR-021).
    Conflicted,
    /// Not tried: the connection went away before this file's turn. Retried next reconnection.
    ///
    /// Distinct from `Failed` although both leave the row. `NotAttempted` is retried and `Failed`
    /// is not, so a developer told their work failed when it is merely queued has been told
    /// something false.
    NotAttempted,
    /// Tried, and could not be completed for a reason that will not fix itself.
    Failed(String),
}

/// What happened to every file, in the order they were retained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub files: Vec<(RelPath, Outcome)>,
}

impl ReconcileReport {
    /// Whether anything needs the developer. The conflict panel's cue.
    pub fn has_conflicts(&self) -> bool {
        self.files.iter().any(|(_, o)| o == &Outcome::Conflicted)
    }
}

/// Whether this transition is the one that starts a reconciliation.
///
/// Extracted from the composition root so the rule is testable: a sink there is reached only by
/// starting the application, and three requirements turn on this one comparison.
///
/// - **Once per transition** into `Connected`. The sink is invoked with the current state on
///   subscribe and on every change, so a reconnection reporting `Connected` twice would otherwise
///   reconcile twice -- harmless and still wrong.
/// - **Never on any other state.** `Connecting` and `Retrying` are not connected, and reconciling
///   during them would write against a host that is not there.
/// - **Edge case EC-16**: a reconnection whose protocol version is incompatible never reaches
///   `Connected` (§3.8 refuses a newer engine and redeploys an older one), so it does not reconcile.
///   A workspace that cannot be used cannot be reconciled, and that holds here by construction
///   rather than by a check of its own.
pub fn entered_connected(previous: &ConnectionState, current: &ConnectionState) -> bool {
    current == &ConnectionState::Connected && previous != &ConnectionState::Connected
}

pub struct Reconcile {
    cache: Arc<dyn WorkspaceCache>,
    provider: Arc<dyn WorkspaceProvider>,
    merge: Arc<dyn TextMerge>,
    /// Whether a run is in flight. Two triggers exist, and a workspace opened during a
    /// reconnection's run would otherwise start a second one reading the same rows the first is
    /// writing -- each could write a file the other had already merged.
    running: std::sync::atomic::AtomicBool,
}

impl Reconcile {
    pub fn new(
        cache: Arc<dyn WorkspaceCache>,
        provider: Arc<dyn WorkspaceProvider>,
        merge: Arc<dyn TextMerge>,
    ) -> Self {
        Self {
            cache,
            provider,
            merge,
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Attempt every pending edit once.
    ///
    /// Per file, and independently (FR-023): one conflict does not hold back the files that would
    /// have landed. The loop stops early only for a lost connection, because a lost connection
    /// means the remaining attempts would all fail for the same reason and reporting them as
    /// `Failed` would be false -- they were never tried.
    pub async fn run(&self, ws: &WorkspaceId) -> ReconcileReport {
        use std::sync::atomic::Ordering;
        // One run at a time. A second trigger arriving mid-run returns an empty report: the rows
        // it would have attempted are the ones the first run is attempting, and every row it did
        // not reach stays for the next trigger. Nothing is lost by declining.
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return ReconcileReport::default();
        }
        let report = self.run_once(ws).await;
        self.running.store(false, Ordering::Release);
        report
    }

    async fn run_once(&self, ws: &WorkspaceId) -> ReconcileReport {
        let pending = match self.cache.pending_edits(ws) {
            Ok(rows) => rows,
            // Nothing can be reported per file, because the list of files could not be read. This
            // is the one case with no per-file detail to lose.
            Err(e) => {
                return ReconcileReport {
                    files: vec![(RelPath::root(), Outcome::Failed(e.to_string()))],
                }
            }
        };

        let mut files = Vec::with_capacity(pending.len());
        let mut lost = false;
        for (path, edit) in pending {
            if lost {
                files.push((path, Outcome::NotAttempted));
                continue;
            }
            let outcome = self.one(ws, &path, &edit).await;
            // A dropped connection stops the run. Everything after it is `NotAttempted`, which is
            // what makes FR-028's "an interruption loses nothing" visible in the report as well as
            // true in the store.
            if outcome == Outcome::NotAttempted {
                lost = true;
            }
            files.push((path, outcome));
        }
        ReconcileReport { files }
    }

    /// One file. Read the host, decide, write or ask.
    async fn one(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        edit: &crate::application::ports::workspace_cache::PendingEdit,
    ) -> Outcome {
        // A file the client does not hold as text always prompts, whatever the host did (FR-025a).
        // Checked before the read, because reading is pointless when the answer cannot change.
        if !edit.mergeable {
            return Outcome::Conflicted;
        }
        let Some((base_bytes, base_hash)) = edit.base.as_ref() else {
            // No base: the file was created offline, so there is nothing to merge against. A write
            // is either accepted or refused, and a refusal means the host has the path too.
            return self.write(ws, path, &edit.content, None).await;
        };

        let remote = match self.provider.read_file(ws, path, None).await {
            Ok(chunk) => chunk,
            Err(ProviderError::Offline) => return Outcome::NotAttempted,
            // A file the host no longer has is a question for the developer, not a failure: the
            // deletion and the edit both have a claim, and FR-026 says neither wins silently.
            Err(ProviderError::NotFound) => return Outcome::Conflicted,
            Err(e) => return Outcome::Failed(e.to_string()),
        };

        // The fast-forward test is a hash comparison, which is why the hash is stored beside the
        // base content: it never needs a decompression to answer.
        if &remote.sha256 == base_hash {
            return match self.write(ws, path, &edit.content, Some(base_hash)).await {
                Outcome::Merged => Outcome::FastForwarded,
                other => other,
            };
        }

        // Three texts, or the file prompts. A byte sequence that is not UTF-8 cannot be merged as
        // text, and guessing would be the content sniffing research.md rejected.
        let (Ok(base), Ok(local), Ok(remote_text)) = (
            std::str::from_utf8(base_bytes),
            std::str::from_utf8(&edit.content),
            std::str::from_utf8(&remote.bytes),
        ) else {
            return Outcome::Conflicted;
        };

        match self.merge.merge(base, local, remote_text) {
            // Written against the **remote's** hash, not the base's: the host has moved, and a write
            // carrying the old base would be refused as stale for a change we have just accounted
            // for.
            MergeOutcome::Clean(merged) => {
                self.write(ws, path, merged.as_bytes(), Some(&remote.sha256))
                    .await
            }
            MergeOutcome::Conflict => Outcome::Conflicted,
        }
    }

    /// Write, and forget the row **only** on a confirmation.
    ///
    /// The single place `forget_pending` is called. Everything that is not a confirmed write leaves
    /// the row, which is what makes FR-022 a property of this function rather than a rule every
    /// branch has to remember.
    async fn write(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        content: &[u8],
        base: Option<&Sha256>,
    ) -> Outcome {
        // A file created offline has no base to be conditional on. The engine's write is
        // conditional on a digest, so an all-zero digest is used for "there was nothing here":
        // §4.8 gives no other spelling, and a digest of the content would claim the host already
        // holds what we are about to send.
        let empty = Sha256::parse(&"0".repeat(64)).expect("a valid digest shape");
        let base = base.unwrap_or(&empty);
        match self.provider.write_file(ws, path, content, base).await {
            Ok(_) => match self.cache.forget_pending(ws, path) {
                Ok(()) => Outcome::Merged,
                // The host has it and the row could not be dropped. Reported rather than hidden:
                // the next reconciliation will find the row, read the host, see it matches, and
                // fast-forward to the same content -- harmless, and worth knowing about.
                Err(e) => Outcome::Failed(format!("written, but the local row remains: {e}")),
            },
            Err(ProviderError::Offline) => Outcome::NotAttempted,
            // `-32004`. The host moved between this reconciliation's read and its write, which is
            // the host disagreeing and not a failure (FR-020b). The same answer `conflict_resolve`
            // gives for the same race, and the row stays so the developer can settle it.
            Err(ProviderError::WriteConflict) => Outcome::Conflicted,
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }
}
