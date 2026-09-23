//! Answering `auth/handshake` (§4.8).
//!
//! What this engine advertises is what it actually serves. Advertising a capability it does not
//! implement would make the client offer a feature that fails in a developer's hands, which is
//! the failure capability exchange exists to prevent.

use crate::session::SessionRegistry;
use apex_protocol::wire::{CapabilitySet, HandshakeRequest, HandshakeResponse, PROTOCOL_VERSION};

/// Everything this engine can do. F002 serves the session methods and no workspace method;
/// F003 adds those, and adds their tokens here at the same time.
pub fn capabilities() -> CapabilitySet {
    CapabilitySet::of(&["auth/handshake", "session/shutdown", "session/onRestart"])
}

pub fn respond(registry: &SessionRegistry, request: &HandshakeRequest) -> HandshakeResponse {
    // Resumption is honoured only for the identity this engine actually holds. `resumed` is
    // set truthfully because the client's whole ability to tell a re-attachment from a new
    // session depends on it.
    let resumed = request
        .resume_session
        .as_ref()
        .is_some_and(|id| registry.resume(id));

    HandshakeResponse {
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
        capabilities: capabilities(),
        session_id: registry.current(),
        resumed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_protocol::wire::SessionId;

    fn request(resume: Option<&str>) -> HandshakeRequest {
        HandshakeRequest {
            client_version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: CapabilitySet::of(&["auth/handshake"]),
            resume_session: resume.map(|s| SessionId(s.to_string())),
        }
    }

    /// T034. Unknown tokens are ignored rather than rejected — the property that lets a method
    /// be added without incrementing the protocol version, required of both ends rather than
    /// left as a habit.
    #[test]
    fn unknown_capability_tokens_are_ignored_rather_than_rejected() {
        let registry = SessionRegistry::new();
        let mut r = request(None);
        r.capabilities = CapabilitySet::of(&["auth/handshake", "something/fromTheFuture"]);
        let response = respond(&registry, &r);
        assert_eq!(response.protocol_version, PROTOCOL_VERSION);
        assert!(response.capabilities.has("auth/handshake"));
        assert!(
            !response.capabilities.has("something/fromTheFuture"),
            "the engine must not claim a capability it does not have"
        );
    }

    /// The engine advertises only what it serves. A capability advertised and missing is a
    /// feature that fails in a developer's hands.
    #[test]
    fn nothing_is_advertised_that_this_engine_does_not_serve() {
        let c = capabilities();
        assert!(c.has("auth/handshake"));
        assert!(
            !c.has("workspace/readFile"),
            "F002 serves no workspace method; F003 adds them and their tokens together"
        );
    }

    #[test]
    fn a_first_connect_is_not_reported_as_resumed() {
        let registry = SessionRegistry::new();
        assert!(!respond(&registry, &request(None)).resumed);
    }

    /// FR-024c. An identity the engine does not hold yields `resumed: false` and a session id
    /// that is this engine's — never a silent pretence that the old one continued.
    #[test]
    fn an_unrecognised_identity_yields_a_new_session_and_says_so() {
        let registry = SessionRegistry::new();
        let response = respond(&registry, &request(Some("from-a-dead-engine")));
        assert!(!response.resumed, "the client must learn its work is gone");
        assert_eq!(response.session_id, registry.current());
        assert_ne!(response.session_id, SessionId("from-a-dead-engine".into()));
    }

    #[test]
    fn the_identity_this_engine_holds_resumes() {
        let registry = SessionRegistry::new();
        let id = registry.current();
        let response = respond(&registry, &request(Some(&id.0)));
        assert!(response.resumed);
        assert_eq!(response.session_id, id);
    }
}
