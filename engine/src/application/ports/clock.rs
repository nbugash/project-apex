//! Outbound port: time.
//!
//! Milliseconds as a number rather than `Instant`, because a fake clock then costs one field
//! and the volume requirements become arithmetic. "A thousand writes in one second yields at
//! most ten events" is a test that runs instantly against a settable counter and a test that
//! sleeps for a second against a real one -- and the sleeping version is the one that gets
//! marked flaky and then ignored.

/// Monotonic milliseconds. The origin is arbitrary; only differences mean anything.
pub type Millis = u64;

pub trait Clock: Send {
    fn now(&self) -> Millis;
}
