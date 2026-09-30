//! Try again, on a backoff, until the connection returns or the failure says it will not (§11.5).
//!
//! **This loop did not exist.** `supervise.rs` has held the policy since F001 and the transport has
//! had `report_retrying` to publish the waits, and nothing ran either: after a loss the state stayed
//! `Disconnected` for the life of the process. F012 found it because reconciliation, which runs on
//! reconnection, never ran (FR-018a).

use crate::application::ports::connection::Reconnectable;
use crate::application::use_cases::supervise::{response_to, Backoff, Response};
use crate::domain::failure::FailureCondition;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How a run of the loop ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reconnected {
    /// Back, after this many attempts.
    Connected { attempts: u32 },
    /// Stopped, because another attempt would fail the same way -- a changed host key, a refused
    /// credential. The developer has to act, and a loop that kept trying would only hide that.
    GaveUp(FailureCondition),
}

/// One loop at a time.
///
/// Guarded because a failed attempt publishes `Disconnected` again, which is the very state that
/// starts a loop -- without the guard each failure would start another one, and the backoff each
/// loop kept would be meaningless.
#[derive(Default)]
pub struct Reconnect {
    running: AtomicBool,
}

impl Reconnect {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run until connected or given up. `None` if a loop is already running.
    ///
    /// `sleep` and `jitter` are parameters so the tests run in microseconds and can choose the
    /// random draw; the composition root passes `std::thread::sleep` and a draw from the standard
    /// library's `RandomState`.
    pub fn run(
        &self,
        target: &dyn Reconnectable,
        sleep: &dyn Fn(Duration),
        jitter: &mut dyn FnMut() -> f64,
    ) -> Option<Reconnected> {
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let outcome = attempt_until_settled(target, sleep, jitter);
        self.running.store(false, Ordering::Release);
        Some(outcome)
    }
}

fn attempt_until_settled(
    target: &dyn Reconnectable,
    sleep: &dyn Fn(Duration),
    jitter: &mut dyn FnMut() -> f64,
) -> Reconnected {
    // A fresh schedule per loss: the previous outage's backoff says nothing about this one, and
    // `Backoff::reset` exists for exactly the moment a connection comes back.
    let mut backoff = Backoff::default();
    loop {
        let delay = backoff.next_delay(jitter());
        // Published before the wait, so the status bar says "trying again in N seconds" for the
        // whole of it rather than after it has passed. Rounded up: a wait of 0.4 s reported as
        // "in 0 seconds" reads as a hang.
        target.report_retrying(backoff.attempt(), delay.as_secs_f64().ceil() as u64);
        sleep(delay);
        match target.reconnect() {
            Ok(()) => {
                return Reconnected::Connected {
                    attempts: backoff.attempt(),
                }
            }
            Err(_) => match target.last_failure() {
                // Unclassifiable is worth another try: a child killed by a signal has no code,
                // and that is what an engine that was stopped looks like.
                None => continue,
                // `false`: a loop cannot put a passphrase prompt in front of the developer, so an
                // authentication failure is a report rather than an assisted retry.
                Some(condition) => match response_to(condition, false) {
                    Response::RetryAfterBackoff => continue,
                    _ => return Reconnected::GaveUp(condition),
                },
            },
        }
    }
}

/// A uniform draw in `[0, 1)` from the standard library, for the backoff's jitter.
///
/// `RandomState` is seeded per instance from the operating system, which is what jitter needs --
/// clients spread out, not a reproducible sequence -- and it costs no dependency.
pub fn jitter() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u8(0);
    (h.finish() >> 11) as f64 / (1u64 << 53) as f64
}

/// What the reconnection sequence does to the current workspace, as three steps.
///
/// A trait with one production implementation, and the reason is its order: the sequence exists to
/// run these **in this order**, and a recording fake is the only way to assert an order rather than
/// an outcome. Reconciling before the new engine has registered the workspace reads nothing and
/// reports every file `Failed`.
#[async_trait::async_trait]
pub trait Resume: Send + Sync {
    /// Tell the new engine about the workspace. `false` if it did not answer.
    async fn register(&self, ws: &crate::domain::workspace::WorkspaceId) -> bool;
    /// Tell the new engine again what to watch. It forgot with the old connection; the client did
    /// not, so this is the client telling it rather than asking what it had.
    async fn rewatch(&self, ws: &crate::domain::workspace::WorkspaceId);
    /// Ask for git status, which is also what makes the new engine watch the repository again.
    async fn refresh_git(&self, ws: &crate::domain::workspace::WorkspaceId);
    /// Send the work the host has not seen.
    async fn reconcile(&self, ws: &crate::domain::workspace::WorkspaceId);
    /// Cache what the developer is likely to want next time the connection goes (FR-029b's second
    /// trigger). Last: it is speculative, and the developer's own work lands before it.
    async fn prefetch(&self, ws: &crate::domain::workspace::WorkspaceId);
}

/// After a reconnection: register, re-watch, refresh git, reconcile, then prefetch -- §11.5's order,
/// which puts reconciling offline work after everything it depends on, and prefetch after that. Re-watching comes before git because git status's own
/// refresh is nudged by file events (A-GITNUDGE), which only arrive for watched paths.
///
/// Stops after a failed registration (EC-16). A workspace the engine cannot take -- an incompatible
/// protocol, a root that is gone -- cannot be reconciled, and trying would only turn every pending
/// file into a failure the developer can do nothing with.
pub async fn resume_after_reconnect(
    resume: &dyn Resume,
    ws: &crate::domain::workspace::WorkspaceId,
) -> bool {
    if !resume.register(ws).await {
        return false;
    }
    resume.rewatch(ws).await;
    resume.refresh_git(ws).await;
    resume.reconcile(ws).await;
    resume.prefetch(ws).await;
    true
}
