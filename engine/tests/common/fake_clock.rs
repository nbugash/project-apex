//! Time as a settable number, shareable across threads.
//!
//! The whole reason `Clock` is a port. "A thousand writes in one second" against this runs
//! instantly and deterministically; against a real clock it sleeps for a second, goes flaky
//! under load, and gets marked ignored.
//!
//! `Mutex` plus `Condvar` rather than `Cell`, since F010. A `Cell`-backed fake is actively
//! `!Sync` and so cannot be shared with the thread doing the waiting -- and F010's escalation
//! tests advance one clock and assert about work happening on another thread, which is the
//! entire shape of what they measure.

use apex_engine::application::ports::clock::{Clock, Millis};
use std::sync::{Condvar, Mutex};

#[derive(Default)]
pub struct FakeClock {
    now: Mutex<Millis>,
    /// Woken on every change, so a waiter blocked on a time passing can be released by
    /// advancing rather than by waiting.
    changed: Condvar,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, by: Millis) {
        let mut now = self.now.lock().expect("fake clock poisoned");
        *now += by;
        // `notify_all` and never `notify_one`. N reader threads and the escalation thread can
        // all be inside `sleep_until` on this one clock, and a single `advance` must release
        // every one of them whose deadline has passed -- waking one leaves the rest parked on
        // a clock that has already moved past their deadlines, which is a hang that looks like
        // a slow test.
        self.changed.notify_all();
    }

    pub fn set(&self, to: Millis) {
        let mut now = self.now.lock().expect("fake clock poisoned");
        *now = to;
        self.changed.notify_all();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Millis {
        *self.now.lock().expect("fake clock poisoned")
    }
}
