//! One request, its priority, and how it ends. See
//! specs/003-ssh-transport-core/data-model.md.
//!
//! The wire format's own constants — the frame cap, the JSON-RPC error codes — are §4 of the
//! system specification's and are expressed once, here and in `domain::failure`. Nothing
//! restates them.

use serde::{Deserialize, Serialize};
use std::fmt;

/// §4.1. A frame beyond this is a protocol error; payloads this large belong on SFTP.
/// Re-exported from `apex-protocol`, which owns it: the cap is part of the wire format, and
/// the engine enforces the same number from the same definition.
pub use apex_protocol::framing::MAX_FRAME_BYTES;

/// §4.4. The application code range is -32000..=-32099.
pub const ERR_PAYLOAD_TOO_LARGE: i32 = -32007;
pub const ERR_CANCELLED: i32 = -32008;
pub const ERR_INTERNAL: i32 = -32603;

/// The default a caller gets when it states no limit.
///
/// Deliberately generous. A limit that fires early turns a slow link into a broken one, and
/// the interaction budget is protected by withdrawal, not by expiry.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Identifies one request for the life of one session.
///
/// Minted by the registry, never by a caller: uniqueness is a property of the thing that
/// would detect a collision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub String);

impl RequestId {
    pub fn new(n: u64) -> Self {
        Self(format!("req_{n}"))
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which class of traffic a request belongs to (§4.6).
///
/// Stated by the caller, never inferred: the same method is interactive when the user opens
/// a file and background when prefetch warms the cache, so the method name cannot decide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    /// Nobody is waiting. Prefetch, indexing status, cache warming.
    Background,
    /// The user is waiting. Ordered ahead of `Background`.
    Interactive,
}

/// Exactly one of these resolves a request. The set is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestOutcome {
    /// A reply carrying a result.
    Answered(String),
    /// The remote side considered the request and refused it, or it could not be sent.
    Failed { code: i32, message: String },
    /// The deadline passed with no reply.
    TimedOut,
    /// The caller withdrew it.
    Withdrawn,
    /// The link died while this request was outstanding.
    ///
    /// Distinct from `Failed` because the caller can usefully retry this one once the link
    /// returns. A `Failed` means the refusal will repeat.
    ConnectionLost,
}

impl RequestOutcome {
    pub fn is_answered(&self) -> bool {
        matches!(self, Self::Answered(_))
    }
}

/// A passphrase.
///
/// A type rather than a `String` because FR-008 is a property that must hold everywhere the
/// value goes, and the only way to guarantee that is to make leaking it require deliberate
/// effort. A `String` passphrase is one `{:?}` away from a log line.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(value.into())
    }

    /// Borrow the bytes to write them somewhere. Deliberately not `Clone`: every copy is
    /// another buffer someone has to remember to zero.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // `write_volatile` so the compiler cannot decide this write is dead and remove it,
        // which it is entitled to do for a buffer nothing reads afterwards.
        for byte in self.0.iter_mut() {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_orders_ahead_of_background() {
        assert!(Priority::Interactive > Priority::Background);
        let mut v = [
            Priority::Background,
            Priority::Interactive,
            Priority::Background,
        ];
        v.sort();
        assert_eq!(v.last(), Some(&Priority::Interactive));
    }

    #[test]
    fn request_ids_are_distinct() {
        assert_ne!(RequestId::new(1), RequestId::new(2));
    }

    /// T010 — FR-008. The point of `Secret` is that leaking it must require effort.
    #[test]
    fn secret_renders_no_plaintext() {
        let s = Secret::new("hunter2");

        assert_eq!(format!("{s:?}"), "Secret(redacted)");
        assert_eq!(format!("{s}"), "<redacted>");
        assert!(!format!("{s:?} {s}").contains("hunter2"));

        // The formatted payload of a panic carrying one must not leak it either — a panic
        // message reaches a crash report, which is a log by another name.
        let panicked =
            std::panic::catch_unwind(|| panic!("connect failed: {:?}", Secret::new("hunter2")));
        let payload = panicked.unwrap_err();
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(
            !text.contains("hunter2"),
            "panic payload leaked the secret: {text}"
        );
    }

    #[test]
    fn secret_exposes_its_bytes_only_deliberately() {
        let s = Secret::new("abc");
        assert_eq!(s.expose(), b"abc");
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
    }
}
