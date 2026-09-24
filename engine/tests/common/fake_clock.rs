//! Time as a settable number.
//!
//! The whole reason `Clock` is a port. "A thousand writes in one second" against this runs
//! instantly and deterministically; against a real clock it sleeps for a second, goes flaky
//! under load, and gets marked ignored.

use apex_engine::application::ports::clock::{Clock, Millis};
use std::cell::Cell;

#[derive(Default)]
pub struct FakeClock {
    now: Cell<Millis>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, by: Millis) {
        self.now.set(self.now.get() + by);
    }

    pub fn set(&self, to: Millis) {
        self.now.set(to);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Millis {
        self.now.get()
    }
}
