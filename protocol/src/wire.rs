//! The messages the client and engine exchange, from §4.8.
//!
//! Shared rather than duplicated: a field name that differs between the two ends fails at the
//! far end, where the evidence is worst.

use serde::{Deserialize, Serialize};

/// What this build of the protocol speaks.
///
/// Incremented on a **breaking** change only — never for an added method, an added optional
/// parameter or an added result field. A version that increments on additions forces a
/// redeployment across every host for changes that needed none, and it is only safe because
/// both ends are required to ignore what they do not recognise.
pub const PROTOCOL_VERSION: u32 = 1;

/// An opaque session identity, minted by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What one side can do: opaque tokens, compared by exact match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet(pub Vec<String>);

impl CapabilitySet {
    pub fn of(tokens: &[&str]) -> Self {
        Self(tokens.iter().map(|t| t.to_string()).collect())
    }

    /// Exact match, never a prefix. A prefix test would make `workspace/read` satisfy a
    /// requirement for `workspace/readFile`.
    pub fn has(&self, token: &str) -> bool {
        self.0.iter().any(|t| t == token)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeRequest {
    pub client_version: String,
    pub protocol_version: u32,
    pub capabilities: CapabilitySet,
    /// Present when re-attaching after a disconnection; absent on a first connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_session: Option<SessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeResponse {
    pub engine_version: String,
    pub protocol_version: u32,
    pub capabilities: CapabilitySet,
    pub session_id: SessionId,
    /// False means a new session was created. The client must surface that rather than treat
    /// it as success — a client that silently continues shows work that is not happening.
    pub resumed: bool,
}

/// Sent by the engine after it re-executes itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartNotice {
    pub session_id: SessionId,
    /// Everything that did not survive. Empty is a positive assertion that nothing was lost,
    /// not an absence of information.
    pub unpreserved: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_match_exactly_and_never_by_prefix() {
        let c = CapabilitySet::of(&["workspace/readFile", "session/shutdown"]);
        assert!(c.has("workspace/readFile"));
        assert!(
            !c.has("workspace/read"),
            "a prefix must not satisfy a capability check"
        );
        assert!(!c.has("workspace/readFileExtra"));
        assert!(!c.has(""));
    }

    #[test]
    fn an_absent_resume_session_is_omitted_from_the_wire() {
        let r = HandshakeRequest {
            client_version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: CapabilitySet::default(),
            resume_session: None,
        };
        let json = serde_json::to_string(&r).expect("serialise");
        assert!(!json.contains("resume_session"), "{json}");
    }
}
