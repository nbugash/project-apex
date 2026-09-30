//! Outbound port: supply connection state and its transitions.

use crate::domain::connection::ConnectionState;

pub type StateSink = Box<dyn Fn(ConnectionState) + Send + Sync>;

pub trait ConnectionStatusSource: Send + Sync {
    fn current(&self) -> ConnectionState;

    /// The sink is invoked once with the current state, then on every transition — so a
    /// subscriber never has to poll for its initial value.
    fn subscribe(&self, sink: StateSink);
}

/// A connection that can be attempted again after it was lost (§11.5, F012's FR-018a).
///
/// A port so the reconnection loop is tested against a fake rather than by spawning processes:
/// the policy -- how long to wait, when to give up -- is what the tests are about, and a real
/// transport would make every one of them slow and dependent on a machine's `ssh`.
pub trait Reconnectable: Send + Sync {
    /// One attempt. Blocking: the transport's own `connect` waits out the window that tells an
    /// established child from one that gave up, and the loop runs on a thread of its own.
    fn reconnect(&self) -> Result<(), String>;

    /// Why the last attempt ended, when that can be told. `None` when it cannot -- a child killed
    /// by a signal has no exit code to classify -- which the loop treats as worth another try.
    fn last_failure(&self) -> Option<crate::domain::failure::FailureCondition>;

    /// Publish that the loop is waiting before its next attempt.
    fn report_retrying(&self, attempt: u32, next_in_secs: u64);
}
