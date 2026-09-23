//! Outbound port: the first exchange on a session.
//!
//! Guarantees are in specs/004-daemon-bootstrap/contracts/handshake.md.

use apex_protocol::wire::{HandshakeRequest, HandshakeResponse};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HandshakeError {
    /// The engine started but never answered. Distinct from a transport failure, because
    /// "the engine is wedged" and "the connection dropped" have different remedies.
    #[error("the engine did not answer the handshake")]
    TimedOut,
    #[error("the engine's reply could not be understood: {0}")]
    Malformed(String),
    #[error("the connection failed before the handshake completed")]
    ConnectionLost,
}

#[allow(async_fn_in_trait)]
pub trait HandshakePeer: Send + Sync {
    async fn handshake(
        &self,
        request: HandshakeRequest,
    ) -> Result<HandshakeResponse, HandshakeError>;
}
