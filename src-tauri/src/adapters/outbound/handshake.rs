//! The handshake, carried over F001's transport.
//!
//! The adapter between `HandshakePeer` and `RequestTransport`: it turns a typed handshake into
//! a JSON-RPC request, sends it on the established connection, and turns the reply back into a
//! typed response.
//!
//! Placement note: design.md put this in the deploy module. It lives here instead, because it
//! has nothing to do with moving artifacts — it speaks the protocol over a connection that
//! already exists, which is a different job from putting a binary on a host.

use crate::application::ports::handshake::{HandshakeError, HandshakePeer};
use crate::application::ports::transport::{Request, RequestTransport};
use crate::domain::request::RequestOutcome;
use apex_protocol::wire::{HandshakeRequest, HandshakeResponse};

pub struct TransportHandshake<'a, T: RequestTransport> {
    transport: &'a T,
}

impl<'a, T: RequestTransport> TransportHandshake<'a, T> {
    pub fn new(transport: &'a T) -> Self {
        Self { transport }
    }
}

impl<T: RequestTransport> HandshakePeer for TransportHandshake<'_, T> {
    async fn handshake(
        &self,
        request: HandshakeRequest,
    ) -> Result<HandshakeResponse, HandshakeError> {
        let params = serde_json::to_string(&request)
            .map_err(|e| HandshakeError::Malformed(format!("could not encode the request: {e}")))?;

        // Interactive: nothing else can proceed until this completes, so it must not queue
        // behind background work (A-PRI).
        let outcome = self
            .transport
            .send(Request::interactive("auth/handshake", params))
            .await;

        match outcome {
            RequestOutcome::Answered(body) => parse_response(&body),
            // Distinct from a transport failure by design. "The engine never answered" and
            // "the connection dropped" have different remedies, and F001 already paid for
            // blurring failure conditions once.
            RequestOutcome::TimedOut => Err(HandshakeError::TimedOut),
            RequestOutcome::ConnectionLost => Err(HandshakeError::ConnectionLost),
            RequestOutcome::Withdrawn => Err(HandshakeError::ConnectionLost),
            RequestOutcome::Failed { code, message } => Err(HandshakeError::Malformed(format!(
                "the engine refused the handshake ({code}): {message}"
            ))),
        }
    }
}

/// Pull the typed response out of a JSON-RPC reply.
///
/// The engine's reply is untrusted input like any other (Principle VI): a reply that is not
/// what this client expects produces a named error, never a panic and never a partially
/// populated response.
fn parse_response(body: &str) -> Result<HandshakeResponse, HandshakeError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| HandshakeError::Malformed(format!("the reply is not JSON: {e}")))?;

    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("no message");
        return Err(HandshakeError::Malformed(format!(
            "the engine refused the handshake ({code}): {message}"
        )));
    }

    let result = value
        .get("result")
        .ok_or_else(|| HandshakeError::Malformed("the reply carries no result".into()))?;

    serde_json::from_value(result.clone())
        .map_err(|e| HandshakeError::Malformed(format!("the reply is not a handshake: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_reply_parses() {
        let body = r#"{"jsonrpc":"2.0","id":"1","result":{
            "engine_version":"0.1.0","protocol_version":1,
            "capabilities":["auth/handshake"],"session_id":"s-1","resumed":false}}"#;
        let r = parse_response(body).expect("parses");
        assert_eq!(r.protocol_version, 1);
        assert_eq!(r.session_id.0, "s-1");
        assert!(!r.resumed);
        assert!(r.capabilities.has("auth/handshake"));
    }

    /// An engine that refuses says so, and the refusal reaches the caller with its code rather
    /// than becoming a generic parse failure.
    #[test]
    fn an_error_reply_is_reported_with_its_code() {
        let body = r#"{"jsonrpc":"2.0","id":"1","error":{"code":-32602,"message":"bad params"}}"#;
        match parse_response(body) {
            Err(HandshakeError::Malformed(m)) => {
                assert!(m.contains("-32602"), "{m}");
                assert!(m.contains("bad params"), "{m}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// Principle VI. Whatever the engine sends is untrusted: none of these may panic, and none
    /// may produce a response with invented fields.
    #[test]
    fn a_reply_that_is_not_a_handshake_is_refused_rather_than_guessed() {
        for body in [
            "not json at all",
            "{}",
            r#"{"jsonrpc":"2.0","id":"1"}"#,
            r#"{"jsonrpc":"2.0","id":"1","result":{}}"#,
            r#"{"jsonrpc":"2.0","id":"1","result":{"protocol_version":"one"}}"#,
            r#"{"jsonrpc":"2.0","id":"1","result":[1,2,3]}"#,
        ] {
            assert!(
                matches!(parse_response(body), Err(HandshakeError::Malformed(_))),
                "must be refused: {body}"
            );
        }
    }
}
