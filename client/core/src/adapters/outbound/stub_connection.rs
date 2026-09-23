//! Development and test source for connection state.
//!
//! F001 replaces this adapter with the real transport. Nothing else changes — that swap is
//! the property Principle VIII exists to buy.

use crate::application::ports::connection::{ConnectionStatusSource, StateSink};
use crate::domain::connection::ConnectionState;
use std::sync::{Mutex, RwLock};

#[derive(Default)]
pub struct StubConnectionStatusSource {
    state: RwLock<ConnectionState>,
    sinks: Mutex<Vec<StateSink>>,
}

impl StubConnectionStatusSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drives a transition. Used by the development control and by tests.
    pub fn set(&self, next: ConnectionState) {
        *self.state.write().expect("connection state lock") = next;
        for sink in self.sinks.lock().expect("sink lock").iter() {
            sink(next);
        }
    }
}

impl ConnectionStatusSource for StubConnectionStatusSource {
    fn current(&self) -> ConnectionState {
        *self.state.read().expect("connection state lock")
    }

    fn subscribe(&self, sink: StateSink) {
        // Invoked once immediately, so a subscriber never polls for its initial value.
        sink(self.current());
        self.sinks.lock().expect("sink lock").push(sink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn subscriber_receives_current_state_immediately() {
        let src = StubConnectionStatusSource::new();
        src.set(ConnectionState::Connected);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        src.subscribe(Box::new(move |st| s.lock().unwrap().push(st)));
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[ConnectionState::Connected]
        );
    }

    #[test]
    fn subscribers_receive_every_transition() {
        let src = StubConnectionStatusSource::new();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        src.subscribe(Box::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        src.set(ConnectionState::Connecting);
        src.set(ConnectionState::Connected);
        src.set(ConnectionState::Disconnected);
        assert_eq!(
            count.load(Ordering::SeqCst),
            4,
            "initial + three transitions"
        );
    }
}
