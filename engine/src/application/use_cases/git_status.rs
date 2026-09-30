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
pub fn status_or_nothing(git: &dyn Git, root: &ResolvedPath) -> Result<StatusSnapshot, GitFailure> {
    match git.status(root) {
        Ok(s) => Ok(s),
        Err(e) if e.is_absence() => Ok(StatusSnapshot::nothing()),
        Err(e) => Err(e),
    }
}

/// Shared handle for the parts of the engine that hold one of these per workspace.
pub type SharedPager = Arc<StatusPager>;

/// What the dispatch layer reaches git through: one place holding the adapter and the pager.
///
/// Bundled rather than passed as two parameters because they are only ever used together, and a
/// dispatch arm that could reach a pager without the git that filled it is a shape with no
/// meaning.
pub struct GitService {
    git: Arc<dyn Git>,
    pager: StatusPager,
}

impl GitService {
    pub fn new(git: Arc<dyn Git>) -> Self {
        Self {
            git,
            pager: StatusPager::new(),
        }
    }

    /// Compute a status and hold it, returning the first page.
    ///
    /// An absence — not a repository, or no git — is a successful empty answer rather than an
    /// error, so a workspace that does not use git is fully usable (FR-027, FR-028).
    pub fn refresh(
        &self,
        workspace: &str,
        root: &ResolvedPath,
        limit: u32,
    ) -> Result<GitStatusResult, GitFailure> {
        let snapshot = status_or_nothing(self.git.as_ref(), root)?;
        Ok(self.pager.hold(workspace, snapshot, limit))
    }

    /// A later page of a status already computed.
    pub fn page(
        &self,
        workspace: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<GitStatusResult, PageRefusal> {
        self.pager.page(workspace, cursor, limit)
    }

    /// Which lines of one file differ. An absence yields an empty diff for the same reason a
    /// status does: a workspace without git is not a failing workspace.
    pub fn file_diff(
        &self,
        root: &ResolvedPath,
        relative: &str,
    ) -> Result<apex_protocol::wire::GitDiffResult, GitFailure> {
        match self.git.file_diff(root, relative) {
            Ok(d) => Ok(d),
            Err(e) if e.is_absence() => Ok(apex_protocol::wire::GitDiffResult::default()),
            Err(e) => Err(e),
        }
    }

    /// The files recent commits touched, bounded by commit count and by the frame cap.
    ///
    /// `commits` is already validated as positive; above `MAX_RECENT_COMMITS` it is capped
    /// (guarantee 2). An absence is an empty list (guarantee 5). A list that would not fit in one
    /// frame is truncated (guarantee 6): prefetch is speculative, so a partial answer is a partial
    /// prefetch rather than an error, and a cursor would let a client walk a monorepo's history.
    pub fn recently_changed(
        &self,
        root: &ResolvedPath,
        commits: u32,
    ) -> Result<apex_protocol::wire::RecentlyChangedResult, GitFailure> {
        let commits = commits.min(apex_protocol::wire::MAX_RECENT_COMMITS);
        let paths = match self.git.recently_changed(root, commits) {
            Ok(p) => p,
            Err(e) if e.is_absence() => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(apex_protocol::wire::RecentlyChangedResult {
            paths: within_one_frame(paths),
        })
    }

    pub fn git_dir(&self, root: &ResolvedPath) -> Result<std::path::PathBuf, GitFailure> {
        self.git.git_dir(root)
    }

    pub fn forget(&self, workspace: &str) {
        self.pager.forget(workspace);
    }
}

/// Room left in a frame for the paths themselves: the cap less the JSON-RPC envelope around them.
/// Generous on purpose -- an id is the client's to choose -- because a reply that exceeds the cap
/// is not sent at all, which is worse than one that carries a few paths fewer.
const RECENT_ENVELOPE_HEADROOM: usize = 4 * 1024;

/// Keep paths, in order, while their encoded form fits in one frame.
///
/// Measured as each path's JSON encoding plus its separator, so an escaped character costs what it
/// costs on the wire rather than what it costs in memory.
fn within_one_frame(paths: Vec<String>) -> Vec<String> {
    let budget = apex_protocol::framing::MAX_FRAME_BYTES - RECENT_ENVELOPE_HEADROOM;
    let mut used = 0usize;
    paths
        .into_iter()
        .take_while(|p| {
            used += serde_json::to_string(p)
                .map(|s| s.len())
                .unwrap_or(usize::MAX / 2)
                + 1;
            used <= budget
        })
        .collect()
}
