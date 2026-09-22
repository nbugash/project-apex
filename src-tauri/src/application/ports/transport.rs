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
/// A request that has been sent and is waiting for its outcome.
pub type Pending = std::pin::Pin<Box<dyn std::future::Future<Output = RequestOutcome> + Send>>;

#[allow(async_fn_in_trait)]
pub trait RequestTransport: Send + Sync {
    /// Exactly one outcome, exactly once. Never panics.
    async fn send(&self, request: Request) -> RequestOutcome;

    /// Send, and hand back the id before the outcome.
    ///
    /// `send` cannot serve a caller that may need to withdraw: by the time it returns there
    /// is nothing left to withdraw. A completion request superseded by the next keystroke
    /// needs its id while it is still in flight, which is the only moment `withdraw` is
    /// meaningful — and the contract says the caller provides that id, so something has to
    /// give them one.
    fn begin(&self, request: Request) -> (RequestId, Pending);

    /// Abandon a request in flight. An unknown or already-resolved id is a no-op — the
    /// caller races the reply by nature, and losing that race must not be an error.
    fn withdraw(&self, id: &RequestId);

    /// Returns immediately; never blocks waiting for a connection.
    fn state(&self) -> ConnectionState;
}
