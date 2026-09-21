//! Bridge connection-source transitions to interface events.

use crate::application::ports::connection::{ConnectionStatusSource, StateSink};
use crate::domain::connection::ConnectionState;
use std::sync::Arc;

pub struct ObserveConnection {
    source: Arc<dyn ConnectionStatusSource>,
}

impl ObserveConnection {
    pub fn new(source: Arc<dyn ConnectionStatusSource>) -> Self {
        Self { source }
    }

    pub fn current(&self) -> ConnectionState {
        self.source.current()
    }

    pub fn start(&self, sink: StateSink) {
        self.source.subscribe(sink);
    }
}
