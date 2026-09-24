//! Outbound port: what a `workspaceId` means.
//!
//! In memory, for the engine's lifetime. It does not survive a crash, so a client reconnecting to
//! a restarted engine re-registers — `session/onRestart`'s `unpreserved` list is how it learns it
//! must (F002, §4.8).

use crate::domain::path::CanonicalRoot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootError {
    /// Never registered (`-32001`). The client's answer is to register it.
    NotRegistered,
    /// Registered, and the directory is gone (`-32009`). The client's answer is to tell the
    /// developer and stop presenting its projection as a live view.
    Gone,
    /// Already registered against a different path. Two meanings for one identity is exactly what
    /// `workspaceId` exists to prevent, so this is an error rather than a silent re-point.
    Conflict { existing: String },
    /// Not a directory, or unreadable. Refused at registration rather than on the first read, so
    /// the failure names the workspace instead of a file inside it.
    Unusable,
}

pub trait WorkspaceRoots: Send + Sync {
    /// Idempotent for the same path; `Conflict` for a different one.
    fn register(&self, id: &str, path: &str) -> Result<CanonicalRoot, RootError>;
    /// The root, re-checked: a registered root whose directory has since been deleted is `Gone`,
    /// which the caller must not conflate with a missing path inside it.
    fn resolve(&self, id: &str) -> Result<CanonicalRoot, RootError>;
}
