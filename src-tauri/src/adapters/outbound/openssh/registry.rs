//! Correlation: which reply belongs to which request.
//!
//! The core of the transport, and the first component the system specification says to build
//! (§4.3). Its one guarantee is that an outcome reaches the request that asked for it, and
//! its one hazard is retaining entries for requests that have finished — a leak that grows
//! with session length rather than announcing itself.

use crate::domain::request::{RequestId, RequestOutcome};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::oneshot;

/// Waits for one request's outcome.
pub type Awaiting = oneshot::Receiver<RequestOutcome>;

#[derive(Default)]
pub struct Registry {
    pending: Mutex<HashMap<RequestId, oneshot::Sender<RequestOutcome>>>,
    next: AtomicU64,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint an id and install its receiver.
    ///
    /// The transport must call this **before** writing the frame (FR-011). A reply arriving
    /// the instant the write completes is then still matched; the other order has a window
    /// in which a correct reply is discarded as unknown.
    pub fn register(&self) -> (RequestId, Awaiting) {
        let id = RequestId::new(self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("registry lock")
            .insert(id.clone(), tx);
        (id, rx)
    }

    /// Resolve one request. Returns false if the id is unknown or already resolved.
    ///
    /// False is normal, not exceptional: a request that timed out or was withdrawn may still
    /// be answered, and that late reply is discarded without disturbing anything in flight.
    pub fn resolve(&self, id: &RequestId, outcome: RequestOutcome) -> bool {
        let Some(tx) = self.pending.lock().expect("registry lock").remove(id) else {
            return false;
        };
        // An error here means the caller stopped waiting. The entry is gone either way,
        // which is the property that matters.
        tx.send(outcome).is_ok()
    }

    /// Resolve every outstanding request with the same outcome, and empty the registry.
    ///
    /// Called when the connection dies. Nothing is carried across a reconnect: the remote
    /// side has no memory of a request issued on a dead connection, so a "resumed" request
    /// would wait forever for a reply nobody will send.
    pub fn fail_all(&self, outcome: RequestOutcome) -> usize {
        let drained: Vec<_> = {
            let mut guard = self.pending.lock().expect("registry lock");
            guard.drain().collect()
        };
        let n = drained.len();
        for (_, tx) in drained {
            let _ = tx.send(outcome.clone());
        }
        n
    }

    /// How many requests are outstanding. The measurement SC-005 asserts on.
    pub fn len(&self) -> usize {
        self.pending.lock().expect("registry lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_outcome_reaches_the_request_that_asked() {
        let r = Registry::new();
        let (a, wait_a) = r.register();
        let (b, wait_b) = r.register();
        assert_ne!(a, b);

        // Resolved out of order, which is the case that matters.
        r.resolve(&b, RequestOutcome::Answered("B".into()));
        r.resolve(&a, RequestOutcome::Answered("A".into()));

        assert_eq!(wait_a.await.unwrap(), RequestOutcome::Answered("A".into()));
        assert_eq!(wait_b.await.unwrap(), RequestOutcome::Answered("B".into()));
    }

    #[tokio::test]
    async fn resolving_removes_the_entry() {
        let r = Registry::new();
        let (id, _wait) = r.register();
        assert_eq!(r.len(), 1);
        assert!(r.resolve(&id, RequestOutcome::TimedOut));
        assert_eq!(r.len(), 0, "a resolved request must not be retained");
    }

    #[tokio::test]
    async fn a_second_reply_for_the_same_id_is_discarded() {
        let r = Registry::new();
        let (id, wait) = r.register();
        assert!(r.resolve(&id, RequestOutcome::Answered("first".into())));
        assert!(
            !r.resolve(&id, RequestOutcome::Answered("second".into())),
            "resolving twice must be impossible by construction"
        );
        assert_eq!(
            wait.await.unwrap(),
            RequestOutcome::Answered("first".into())
        );
    }

    #[tokio::test]
    async fn an_unknown_id_resolves_nothing() {
        let r = Registry::new();
        assert!(!r.resolve(
            &RequestId("never-registered".into()),
            RequestOutcome::TimedOut
        ));
    }

    #[tokio::test]
    async fn losing_the_connection_resolves_everything_outstanding() {
        let r = Registry::new();
        let waits: Vec<_> = (0..5).map(|_| r.register().1).collect();
        assert_eq!(r.len(), 5);

        assert_eq!(r.fail_all(RequestOutcome::ConnectionLost), 5);
        assert!(r.is_empty(), "nothing may survive a lost connection");

        for w in waits {
            assert_eq!(w.await.unwrap(), RequestOutcome::ConnectionLost);
        }
    }

    /// SC-005 in miniature. A leak does not fail a short test, so this asserts on the
    /// registry's size after a sustained run rather than on the run completing.
    #[tokio::test]
    async fn nothing_accumulates_across_many_requests() {
        let r = Registry::new();
        for i in 0..2000 {
            let (id, _wait) = r.register();
            let outcome = match i % 3 {
                0 => RequestOutcome::Answered("ok".into()),
                1 => RequestOutcome::TimedOut,
                _ => RequestOutcome::Withdrawn,
            };
            r.resolve(&id, outcome);
        }
        assert_eq!(
            r.len(),
            0,
            "the registry retained entries for finished requests"
        );
    }
}
