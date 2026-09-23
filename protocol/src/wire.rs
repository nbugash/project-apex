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

/// Application error codes from §4.4. Both ends must agree, and neither may write the integer
/// inline — a literal `-32001` in a match arm is a fact stated twice.
pub mod codes {
    /// The engine has never been told about this workspace. The client's answer is to register
    /// it, which is also how it recovers after an engine restart.
    pub const WORKSPACE_NOT_REGISTERED: i32 = -32001;
    /// Path escapes the workspace root (§4.7). Returned identically whether or not the escaped
    /// target exists, so a refusal cannot be used to probe the host's filesystem.
    pub const PATH_REFUSED: i32 = -32002;
    /// Inside the root, and absent. Information the caller is entitled to.
    pub const NOT_FOUND: i32 = -32003;
    /// Frame exceeds §4.1's cap.
    pub const PAYLOAD_TOO_LARGE: i32 = -32007;
    /// Registered, but the root directory is gone. **Not** `WORKSPACE_NOT_REGISTERED`: that one
    /// means re-register, and re-registering a deleted root fails on the not-a-directory check,
    /// surfacing a registration error for a deletion.
    pub const WORKSPACE_GONE: i32 = -32009;
}

/// An opaque workspace identity, minted by the client (A-WORKSPACE).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub String);

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What a directory entry is. Serialised lowercase so the wire reads as §4.8 writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Directory,
}

/// One child of a directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FsEntryWire {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: EntryKind,
    pub size: u64,
    /// Unix seconds.
    pub modified: i64,
}

// ---- workspace/register ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterParams {
    pub workspace_id: WorkspaceId,
    /// Absolute, on the engine's host. Untrusted, like every path off the wire.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResult {
    pub name: String,
    pub canonical_path: String,
}

// ---- workspace/readDirectory ----

/// The largest page §4.8 permits, and the default when a caller states none.
///
/// Not a preference: a listing of a hundred thousand entries at roughly a hundred bytes each is
/// an order of magnitude past §4.1's 1 MiB cap, so an unpaged listing is undeliverable rather
/// than merely slow.
pub const MAX_DIRECTORY_PAGE: u32 = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadDirectoryParams {
    pub workspace_id: WorkspaceId,
    pub relative_path: String,
    /// The last `name` of the previous page. Stateless: the engine keeps no iterator, so a page
    /// may be requested at any time, in any order, after any restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Clamped to `MAX_DIRECTORY_PAGE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadDirectoryResult {
    /// Ordered `(type DESC, name ASC)` — directories first, then by name, byte-wise on UTF-8.
    /// **The ordering is contractual**, because the cursor is the last name returned.
    pub items: Vec<FsEntryWire>,
    /// Present exactly when more entries follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---- workspace/stat ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatParams {
    pub workspace_id: WorkspaceId,
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatResult {
    #[serde(rename = "type")]
    pub kind: EntryKind,
    pub size: u64,
    pub modified: i64,
    /// Lowercase hex of the file's whole content. Absent for a directory: there is nothing to
    /// hash and no caller that needs it. This is the sole input to cache validity (§5.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

// ---- workspace/readFile ----

/// The largest raw payload one frame may carry (A-BULKSIZE).
///
/// Below §4.1's 1 MiB cap by design, not by caution: content is base64 on the wire, four bytes
/// out for every three in, so 512 KiB raw becomes roughly 683 KiB encoded and leaves room for the
/// envelope. A threshold at the cap would encode past it and fail on the first frame with a path.
pub const MAX_INLINE_READ: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileParams {
    pub workspace_id: WorkspaceId,
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileResult {
    /// Base64. There is no utf8 path: assuming text corrupts binary content silently, and a
    /// method that sometimes returns text makes every caller branch on it.
    pub content: String,
    pub encoding: String,
    /// The **whole file's** hash, never the returned range's. A caller assembling ranges compares
    /// this across them; a change means the file moved underneath the read.
    pub sha256: String,
    pub total_size: u64,
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
