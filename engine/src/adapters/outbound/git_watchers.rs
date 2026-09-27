//! The path from a repository changing to a client being told, without being asked.
//!
//! Every piece of this existed before this file did — a watch, a coalescer, a pager, a service
//! that runs git — and none of them were connected, so `git/onStatusUpdate` was a type nothing
//! constructed. That is precisely F004's unfixed defect repeated: a notification defined on the
//! wire with no caller reads as done and ships as nothing.
//!
//! **FR-002 and FR-003 are about the absence of a request.** A client that had to poll would
//! satisfy every request/reply test in this feature and none of its requirements, so what is
//! assembled here is the half that cannot be observed from a reply: the engine noticing on its
//! own and speaking first.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::{GitStatusUpdate, WorkspaceId, MAX_GIT_STATUS_PAGE};

use crate::adapters::inbound::rpc::encode_notification;
use crate::adapters::outbound::frame_writer::FrameWriter;
use crate::application::ports::clock::{Clock, Millis};
use crate::application::ports::git_watch::{GitWatch, GitWatchHandle};
use crate::application::use_cases::git_status::{GitService, StatusCoalescer, EDGE_MS};
use crate::domain::path::ResolvedPath;

/// How long the pump sleeps with nothing due.
///
/// Shorter than the coalescing edge, so a burst that settles is noticed within the edge rather
/// than an edge plus a poll. `watch_thread.rs` picked the same number for the same reason.
const IDLE_POLL_MS: Millis = 50;

/// One observed workspace: the watch that wakes it, and the thread that answers.
struct Observed {
    /// Held, not used. Dropping it stops the watch, which is the whole of its interface.
    _watch: Box<dyn GitWatchHandle>,
    alive: Arc<AtomicBool>,
    pump: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Observed {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
    }
}

/// One git watch per observed workspace, made on demand and released on close.
///
/// Mirrors `Watchers` deliberately: the composition root owns it, a workspace is observed the
/// first time a client asks about it, and `forget` releases everything. A reader who knows one
/// knows the other, which is worth more than any saving from a cleverer arrangement.
pub struct GitWatchers {
    observed: Mutex<HashMap<String, Observed>>,
    watch: Option<Arc<dyn GitWatch>>,
    git: Arc<GitService>,
    clock: Arc<dyn Clock>,
    writer: Arc<FrameWriter>,
    codec: FrameCodec,
}

impl GitWatchers {
    /// `watch` is `None` on a host with no watching facility. Observation then does nothing and
    /// `git/getStatus` still answers — FR-027's degradation, applied to the push half: the loss
    /// is that updates must be asked for, not that git stops working.
    pub fn new(
        watch: Option<Arc<dyn GitWatch>>,
        git: Arc<GitService>,
        clock: Arc<dyn Clock>,
        writer: Arc<FrameWriter>,
        codec: FrameCodec,
    ) -> Self {
        Self {
            observed: Mutex::new(HashMap::new()),
            watch,
            git,
            clock,
            writer,
            codec,
        }
    }

    /// What the inbound layer asks git through.
    ///
    /// An accessor rather than three forwarding methods, and rather than a ninth `dispatch`
    /// parameter. The inbound layer needs one git-shaped thing, and a facade that re-declared
    /// `refresh`, `page` and `file_diff` would be a second place each signature is written.
    pub fn service(&self) -> &GitService {
        &self.git
    }

    /// Begin watching this workspace's repository, if it has one and is not watched already.
    ///
    /// Idempotent, because the natural caller is every `git/getStatus` and a second watch on one
    /// repository would double every notification.
    pub fn observe(&self, workspace: &WorkspaceId, root: &ResolvedPath) {
        let Some(watcher) = self.watch.as_ref() else {
            return;
        };
        let mut observed = self.observed.lock().expect("git watchers poisoned");
        if observed.contains_key(&workspace.0) {
            return;
        }
        // A workspace that is not a repository has no git directory to watch. Not an error and
        // not worth a word: it is the ordinary case for most directories (FR-027).
        let Ok(git_dir) = self.git.git_dir(root) else {
            return;
        };

        let coalescer = Arc::new(Mutex::new(StatusCoalescer::new(EDGE_MS)));
        let waker = Arc::clone(&coalescer);
        let clock = Arc::clone(&self.clock);
        let on_change = Box::new(move || {
            // Runs on the watcher's thread and must not block: it records that something
            // happened and returns. Deciding whether that is worth a git run belongs to the
            // pump, which is the only place the edge is understood.
            waker
                .lock()
                .expect("coalescer poisoned")
                .notice(clock.now());
        });
        let Ok(handle) = watcher.watch(&git_dir, on_change) else {
            return;
        };

        let alive = Arc::new(AtomicBool::new(true));
        let pump = spawn_pump(
            workspace.clone(),
            root.clone(),
            coalescer,
            Arc::clone(&alive),
            Arc::clone(&self.git),
            Arc::clone(&self.clock),
            Arc::clone(&self.writer),
            self.codec.clone(),
        );
        observed.insert(
            workspace.0.clone(),
            Observed {
                _watch: handle,
                alive,
                pump: Some(pump),
            },
        );
    }

    /// The workspace closed or its client went away: stop watching and release the snapshot.
    ///
    /// Both, not one. A pager entry kept after the watch stopped holds the largest status that
    /// workspace ever reported, for a workspace nobody is looking at.
    pub fn forget(&self, workspace: &WorkspaceId) {
        self.observed
            .lock()
            .expect("git watchers poisoned")
            .remove(&workspace.0);
        self.git.forget(&workspace.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_pump(
    workspace: WorkspaceId,
    root: ResolvedPath,
    coalescer: Arc<Mutex<StatusCoalescer>>,
    alive: Arc<AtomicBool>,
    git: Arc<GitService>,
    clock: Arc<dyn Clock>,
    writer: Arc<FrameWriter>,
    codec: FrameCodec,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while alive.load(Ordering::Relaxed) {
            // The lock is taken to decide and released before git runs. Holding it across a
            // full-repository status would block the watcher thread's `notice`, and the change
            // it was reporting is exactly the one the follow-up run exists to catch.
            let claimed = {
                let mut c = coalescer.lock().expect("coalescer poisoned");
                c.begin(clock.now())
            };
            if !claimed {
                std::thread::sleep(std::time::Duration::from_millis(IDLE_POLL_MS));
                continue;
            }

            // The first page, because a notification cannot be answered. When more remains the
            // client pulls the rest with `git/getStatus`, and applies nothing until the last
            // page lands (A-GITPAGE, contracts/git-status.md).
            if let Ok(page) = git.refresh(&workspace.0, &root, MAX_GIT_STATUS_PAGE) {
                let update = GitStatusUpdate {
                    workspace_id: workspace.clone(),
                    current_branch: page.current_branch,
                    changes: page.changes,
                    next_cursor: page.next_cursor,
                };
                if let Some(frame) = encode_notification(&codec, "git/onStatusUpdate", &update) {
                    let _ = writer.write_interactive(&frame);
                }
            }
            // Finished whether or not git answered. Leaving the coalescer running after a
            // failure would wedge the workspace: nothing would ever be due again.
            coalescer
                .lock()
                .expect("coalescer poisoned")
                .finish(clock.now());
        }
    })
}
