//! When to ask git, and how to hand the answer over.
//!
//! Two decisions live here and neither touches a filesystem, which is what makes both testable
//! against a settable clock: **when** a burst of index writes has settled enough to be worth one
//! status computation, and **how** a status too large for one frame is served.

use crate::application::ports::clock::Millis;
use crate::application::ports::git::{Git, GitFailure, StatusSnapshot};
use crate::domain::path::ResolvedPath;
use apex_protocol::wire::{GitChange, GitStatusResult, MAX_GIT_STATUS_PAGE};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A-COALESCE's trailing edge, reused rather than re-chosen.
///
/// The **constant** and not the component: `application::coalescer::Coalescer` is built around
/// `RawEvent` and bulk invalidation, and feeding git paths through the machinery whose job is
/// producing file events is the coupling A-GITWATCH's separate watch exists to prevent. One
/// number to understand, two implementations of very different problems.
pub const EDGE_MS: Millis = crate::application::coalescer::WINDOW_MS;

/// What the coalescer wants done next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Nothing pending.
    Idle,
    /// A burst has settled; run git.
    Run,
    /// Something is pending but the edge has not passed. Come back at this time.
    At(Millis),
}

/// Decides **when** to run git, never how.
///
/// A trailing edge alone is not enough, and that is the part worth reading. A rebase writes the
/// index repeatedly over *seconds*, so every 100 ms gap would start another full-repository
/// status. Holding one run in flight and collapsing everything that arrives during it into a
/// single follow-up bounds a burst of any length at **two** computations: the one already
/// running, and one more reflecting everything that happened while it ran.
#[derive(Debug)]
pub struct StatusCoalescer {
    edge: Millis,
    /// When the most recent change was noticed, if one is pending.
    pending_since: Option<Millis>,
    running: bool,
    /// Something changed while a run was in flight. One flag, not a count: the follow-up
    /// reflects the repository's state at the time it runs, however many writes preceded it.
    dirty_while_running: bool,
}

impl StatusCoalescer {
    pub fn new(edge: Millis) -> Self {
        Self {
            edge,
            pending_since: None,
            running: false,
            dirty_while_running: false,
        }
    }

    /// The watch saw something. Cheap, and safe to call as often as the kernel reports.
    pub fn notice(&mut self, now: Millis) {
        if self.running {
            self.dirty_while_running = true;
        } else {
            self.pending_since = Some(now);
        }
    }

    pub fn due(&self, now: Millis) -> Due {
        if self.running {
            return Due::Idle;
        }
        match self.pending_since {
            None => Due::Idle,
            Some(since) if now.saturating_sub(since) >= self.edge => Due::Run,
            Some(since) => Due::At(since + self.edge),
        }
    }

    /// Claim the run. Returns false when there is nothing due or one is already in flight.
    pub fn begin(&mut self, now: Millis) -> bool {
        if self.due(now) != Due::Run {
            return false;
        }
        self.pending_since = None;
        self.running = true;
        self.dirty_while_running = false;
        true
    }

    /// The run finished. Anything noticed while it ran becomes exactly one follow-up.
    pub fn finish(&mut self, now: Millis) {
        self.running = false;
        if self.dirty_while_running {
            self.dirty_while_running = false;
            // Dated now rather than when it was noticed: the follow-up describes the repository
            // after the run, and dating it earlier would make it due immediately and defeat the
            // edge during a long burst.
            self.pending_since = Some(now);
        }
    }
}

/// An opaque position in a snapshot the engine already holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageRefusal {
    /// Unknown, expired, or from another workspace. Never treated as "start from the beginning":
    /// that would assemble one picture from two snapshots.
    UnknownCursor,
}

/// Serves one computed snapshot in frame-sized pieces.
///
/// **Slices a result already in hand**, rather than re-running git per page. A cursor is
/// therefore a position in a completed snapshot, which is what lets a later page be consistent
/// with the first — the guarantee `contracts/git-status.md` makes and a per-page re-run would
/// break invisibly (A-GITPAGE).
#[derive(Default)]
pub struct StatusPager {
    held: Mutex<HashMap<String, Held>>,
}

struct Held {
    snapshot: StatusSnapshot,
    /// Monotonic **across the process**, not per workspace.
    ///
    /// Per-workspace numbering was the first attempt and was wrong: two workspaces each holding
    /// their first snapshot both had generation 1, so a cursor minted for one validated against
    /// the other and returned its pages. `contracts/git-status.md` requires a cursor to be
    /// single-workspace, and a globally unique generation is what makes that true by
    /// construction rather than by also comparing the workspace.
    generation: u64,
}

impl StatusPager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hold a snapshot and return its first page.
    pub fn hold(&self, workspace: &str, snapshot: StatusSnapshot, limit: u32) -> GitStatusResult {
        let mut held = self.held.lock().expect("pager");
        let generation = held.values().map(|h| h.generation).max().unwrap_or(0) + 1;
        held.insert(
            workspace.to_string(),
            Held {
                snapshot,
                generation,
            },
        );
        drop(held);
        self.page(workspace, None, limit)
            .expect("a freshly held snapshot always has a first page")
    }

    /// One page, from `cursor` or the beginning.
    pub fn page(
        &self,
        workspace: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<GitStatusResult, PageRefusal> {
        let held = self.held.lock().expect("pager");
        let Some(entry) = held.get(workspace) else {
            return Err(PageRefusal::UnknownCursor);
        };
        let offset = match cursor {
            None => 0usize,
            Some(c) => {
                let (generation, offset) = parse_cursor(c).ok_or(PageRefusal::UnknownCursor)?;
                if generation != entry.generation {
                    // From an older snapshot of this workspace, or from another workspace
                    // entirely. Either way the pages would not describe one picture.
                    return Err(PageRefusal::UnknownCursor);
                }
                offset
            }
        };

        let limit = limit.clamp(1, MAX_GIT_STATUS_PAGE) as usize;
        let changes: Vec<GitChange> = entry
            .snapshot
            .changes
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        let next = offset + changes.len();
        Ok(GitStatusResult {
            current_branch: entry.snapshot.branch.clone(),
            changes,
            // Present **exactly** when more remain. An empty final page carries none, which is
            // what lets a client know it may commit.
            next_cursor: (next < entry.snapshot.changes.len())
                .then(|| format!("{}:{}", entry.generation, next)),
        })
    }

    /// Forget a workspace's snapshot — it closed, or its client went away.
    pub fn forget(&self, workspace: &str) {
        self.held.lock().expect("pager").remove(workspace);
    }
}

fn parse_cursor(raw: &str) -> Option<(u64, usize)> {
    let (g, o) = raw.split_once(':')?;
    Some((g.parse().ok()?, o.parse().ok()?))
}

/// Ask git, turning an ordinary absence into an ordinary empty answer.
///
/// A workspace that is not a repository and a host without git both become a successful empty
/// status (FR-027, FR-028). Only a genuine failure is reported as one, and even then the caller
/// shows no git state rather than failing the workspace.
pub fn status_or_nothing(
    git: &dyn Git,
    root: &ResolvedPath,
) -> Result<StatusSnapshot, GitFailure> {
    match git.status(root) {
        Ok(s) => Ok(s),
        Err(e) if e.is_absence() => Ok(StatusSnapshot::nothing()),
        Err(e) => Err(e),
    }
}

/// Shared handle for the parts of the engine that hold one of these per workspace.
pub type SharedPager = Arc<StatusPager>;
