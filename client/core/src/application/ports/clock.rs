//! Outbound port: the clock.
//!
//! A port rather than a call to `SystemTime::now()` so retention is exercisable without waiting
//! fourteen days (Principle VIII names the clock explicitly).

pub trait Clock: Send + Sync {
    /// Unix seconds.
    fn now(&self) -> i64;
}
