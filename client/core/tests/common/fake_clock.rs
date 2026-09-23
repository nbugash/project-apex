//! A clock the test owns, so retention is exercisable without waiting fourteen days.

use apex_shell::application::ports::clock::Clock;
use std::sync::atomic::{AtomicI64, Ordering};

pub struct FakeClock(AtomicI64);

impl FakeClock {
    pub fn at(secs: i64) -> Self {
        Self(AtomicI64::new(secs))
    }

    /// Move time forward. Ageing content past the retention window is a matter of arithmetic,
    /// not of sleeping.
    pub fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
