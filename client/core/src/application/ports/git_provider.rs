//! Outbound port: one page of git status, and one file's changed lines.
//!
//! A port rather than a call on the transport, for FR-001's reason: the consumer must not be
//! able to tell whether an engine is on the other side. It is also what lets the page
//! accumulation in `apply_git_status` be tested without a connection -- and accumulation is
//! where the one interesting mistake lives (A-GITPAGE).
//!
//! Errors are `ProviderError`, not a second taxonomy. §4.4's codes are already mapped there,
//! and two enums describing one set of refusals is two places to keep in step.

use crate::application::ports::workspace_provider::ProviderResult;
use crate::domain::workspace::WorkspaceId;
use apex_protocol::wire::{GitDiffResult, GitStatusResult};
use async_trait::async_trait;

#[async_trait]
pub trait GitProvider: Send + Sync {
    /// One page. `cursor` continues a pull; its absence starts a fresh one.
    ///
    /// A cursor is opaque here on purpose: it is the engine's position in a snapshot the engine
    /// holds, and a client that parsed one would be depending on a shape no contract promises.
    async fn status(
        &self,
        workspace: &WorkspaceId,
        cursor: Option<&str>,
    ) -> ProviderResult<GitStatusResult>;

    /// Which lines of one file differ. Coordinates only -- §12.3 gives this no field for
    /// content and there must never be one.
    async fn file_diff(
        &self,
        workspace: &WorkspaceId,
        relative_path: &str,
    ) -> ProviderResult<GitDiffResult>;
}
