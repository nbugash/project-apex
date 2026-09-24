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

    /// Block until `now() >= deadline`.
    ///
    /// The boundary is `>=` everywhere, which is what makes a deadline of 5 000 a kill at
    /// exactly 5 000 rather than at 5 001.
    ///
    /// A port method rather than `std::thread::sleep`, for the reason `now` is a port method:
    /// the escalation from `SIGTERM` to `SIGKILL` has to be testable without waiting five real
    /// seconds, and a test that sleeps is a test that gets marked flaky and then ignored. The
    /// fake implements this against a condvar its `advance` notifies.
    fn sleep_until(&self, deadline: Millis);
}
