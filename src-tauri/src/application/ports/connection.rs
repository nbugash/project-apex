//! Outbound port: supply connection state and its transitions.

use crate::domain::connection::ConnectionState;

pub type StateSink = Box<dyn Fn(ConnectionState) + Send + Sync>;

pub trait ConnectionStatusSource: Send + Sync {
    fn current(&self) -> ConnectionState;

    /// The sink is invoked once with the current state, then on every transition — so a
    /// subscriber never has to poll for its initial value.
    fn subscribe(&self, sink: StateSink);
}
