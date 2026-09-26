//! Outbound port: time.
//!
//! Milliseconds as a number rather than `Instant`, because a fake clock then costs one field
//! and the volume requirements become arithmetic. "A thousand writes in one second yields at
//! most ten events" is a test that runs instantly against a settable counter and a test that
//! sleeps for a second against a real one -- and the sleeping version is the one that gets
//! marked flaky and then ignored.

/// Monotonic milliseconds. The origin is arbitrary; only differences mean anything.
pub type Millis = u64;

/// `Send + Sync`, because F010 shares one clock across threads.
///
/// F004 needed only `Send`: a watcher thread was handed a clock and owned it. F010's chunker
/// reads the clock on every reader thread while the stop path reads it on the dispatch thread,
/// and one instance cannot be `&`-shared across them without `Sync`. Giving each thread its own
/// would defeat every test that advances one clock and asserts about work on another.
pub trait Clock: Send + Sync {
    fn now(&self) -> Millis;
}

// There is deliberately no `sleep_until` here, and there was briefly.
//
// It was added so the escalation thread could wait five seconds for a `SIGKILL`. Implementing
// the thread showed the shape to be wrong: a sleep cannot be interrupted, so a thread parked in
// one cannot be told to shut down, and under a settable clock it parks until a test advances
// time -- which turns every test that builds a task service and drops it into a hang rather
// than a failure.
//
// The thread waits on its own deadline set instead, which `close` can notify, and asks `now()`
// whether a deadline has passed. The clock stays the authority on time and stops being the
// thing that blocks. A port method nothing needs is one somebody will eventually use.
