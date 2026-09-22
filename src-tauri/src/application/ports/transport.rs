//! The port every later feature reaches the remote engine through.
//!
//! Guarantees are stated in specs/003-ssh-transport-core/contracts/transport.md. Two
//! implementations must satisfy them: the OpenSSH-backed transport and the mock.

use crate::domain::connection::ConnectionState;
use crate::domain::request::{Priority, RequestId, RequestOutcome};
use std::time::Duration;

/// What a caller hands in.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Serialised JSON-RPC params. The transport does not interpret them; what is inside a
    /// frame is the business of the feature that sent it.
    pub params: String,
    pub priority: Priority,
    /// `None` takes the default. A caller that knows its own budget states it.
    pub timeout: Option<Duration>,
}

impl Request {
    pub fn interactive(method: impl Into<String>, params: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            params: params.into(),
            priority: Priority::Interactive,
            timeout: None,
        }
    }

    pub fn background(method: impl Into<String>, params: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            params: params.into(),
            priority: Priority::Background,
            timeout: None,
        }
    }
}

/// Send a request, withdraw one, observe the connection.
///
/// `send` returns an outcome rather than a `Result`: a failure is one of the five ways a
/// request ends, not an error the caller must distinguish from them.
#[allow(async_fn_in_trait)]
pub trait RequestTransport: Send + Sync {
    /// Exactly one outcome, exactly once. Never panics.
    async fn send(&self, request: Request) -> RequestOutcome;

    /// Abandon a request in flight. An unknown or already-resolved id is a no-op — the
    /// caller races the reply by nature, and losing that race must not be an error.
    fn withdraw(&self, id: &RequestId);

    /// Returns immediately; never blocks waiting for a connection.
    fn state(&self) -> ConnectionState;
}
