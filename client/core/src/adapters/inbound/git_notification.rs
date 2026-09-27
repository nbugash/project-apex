//! `git/onStatusUpdate` arrives: apply it, then let the webview know.
//!
//! **A decorator on `NotificationSink`, not a second sink.** The transport has one place to
//! deliver frames to, and adding a second would mean two things to keep registered and one that
//! can be forgotten silently -- which is the exact shape of the defect F011 already found on
//! the engine side.
//!
//! # Why the order matters
//!
//! The webview reads git state back through `git_status`, which reads the projection. So the
//! forward happens **after** the replacement is committed: forwarding first would have the
//! interface read the state that the update was about to replace, and it would be right often
//! enough to look like a rare glitch rather than a race.
//!
//! # Why a worker thread
//!
//! `deliver` runs on the transport's reader thread, and applying an update can issue requests
//! for later pages -- whose replies arrive on that same thread. Applying inline would deadlock
//! on any repository large enough to page, which is exactly the case paging exists for. One
//! worker with one runtime, rather than a thread per notification: updates are coalesced by the
//! engine, so there are few, and processing them in order is what keeps the last one the one
//! that wins.

use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;

use apex_protocol::wire::GitStatusUpdate;

use crate::application::ports::notification_sink::NotificationSink;
use crate::application::use_cases::apply_git_status::{ApplyGitStatus, ApplyOutcome};

/// The method this adapter exists for.
pub const GIT_STATUS_UPDATE: &str = "git/onStatusUpdate";

/// How many updates may wait. Small deliberately: the engine coalesces, so a queue that grows
/// means updates are arriving faster than they can be applied, and the useful response to that
/// is to drop the oldest rather than to buffer a history nobody will read.
const QUEUE: usize = 8;

pub struct GitNotifications {
    work: SyncSender<Job>,
    next: Arc<dyn NotificationSink>,
}

struct Job {
    update: GitStatusUpdate,
    method: String,
    body: String,
}

impl GitNotifications {
    pub fn new(apply: Arc<ApplyGitStatus>, next: Arc<dyn NotificationSink>) -> Self {
        let (work, rx) = sync_channel::<Job>(QUEUE);
        let onward = Arc::clone(&next);
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                crate::logging::warn("git status updates will not be applied: no runtime");
                return;
            };
            rt.block_on(async move {
                while let Ok(job) = rx.recv() {
                    match apply.apply(job.update).await {
                        ApplyOutcome::Applied {
                            paths,
                            pages,
                            refused,
                        } => {
                            if refused > 0 {
                                // Never silent. A path the client refused is a disagreement
                                // with our own engine, which is worth knowing about even
                                // though handling it correctly is enough to stay safe.
                                crate::logging::warn(&format!(
                                    "git status: refused {refused} path(s) outside the workspace"
                                ));
                            }
                            crate::logging::info(&format!(
                                "git status: {paths} path(s) in {pages} page(s)"
                            ));
                        }
                        ApplyOutcome::Discarded(why) => {
                            // Logged rather than swallowed: what is on screen is now knowably
                            // out of date, and an unexplained stale tree is the hardest kind
                            // of bug to attribute.
                            crate::logging::warn(&format!("git status not applied: {why:?}"));
                        }
                    }
                    // Only now. The interface reads the projection back, so it must not be
                    // woken before there is anything new in it to read.
                    onward.deliver(&job.method, &job.body);
                }
            });
        });
        Self { work, next }
    }
}

impl NotificationSink for GitNotifications {
    fn deliver(&self, method: &str, body: &str) {
        if method != GIT_STATUS_UPDATE {
            self.next.deliver(method, body);
            return;
        }
        let Some(update) = parse(body) else {
            // Unparseable, so there is nothing to apply. Still forwarded: this layer's failure
            // to read a frame is not a reason to hide it from everything downstream.
            crate::logging::warn("a git status update could not be read");
            self.next.deliver(method, body);
            return;
        };
        let job = Job {
            update,
            method: method.to_string(),
            body: body.to_string(),
        };
        if self.work.try_send(job).is_err() {
            // The queue is full, or the worker is gone. Dropped rather than blocked: blocking
            // here stops the engine's output being drained, which would turn a slow git apply
            // into a stalled terminal.
            crate::logging::warn("a git status update was dropped: the applier is behind");
        }
    }
}

fn parse(body: &str) -> Option<GitStatusUpdate> {
    let frame: serde_json::Value = serde_json::from_str(body).ok()?;
    serde_json::from_value(frame.get("params")?.clone()).ok()
}
