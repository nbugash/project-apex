//! Outbound port: running a task, from the client's side.
//!
//! The seven §4.8 execution calls the client makes, behind one trait so the UI never learns
//! which implementation is active. Two satisfy it: the remote adapter over the transport, and
//! the local one, which refuses everything and says whose job it is (F015).
//!
//! `#[async_trait]` rather than native `async fn`, for `WorkspaceProvider`'s reason: the
//! concrete provider is chosen at runtime, so the trait must be `dyn`-compatible, which native
//! async-fn-in-trait is not.
//!
//! **`async` here and nothing async in the engine is not an inconsistency.** The client is a
//! Tauri application with an existing runtime and a UI that must not block; the engine is a
//! synchronous process reading one pipe, and giving it a runtime would add a dependency to a
//! binary the client ships on every first connect for no behaviour it needs. The two sides have
//! different constraints and each takes the shape its own constraint implies.

use async_trait::async_trait;

use crate::application::ports::workspace_provider::{Owner, ProviderResult};
use crate::domain::workspace::WorkspaceId;

pub use apex_protocol::wire::{
    AttachResult, Pid, RunTaskParams, SignalName, TaskId, TaskSummary, TerminateSignal,
};

/// What a client asks for when starting a task. The wire type, since the client composes the
/// frame and nothing here reshapes it.
pub type StartRequest = RunTaskParams;

/// The §4.8 execution surface, as the client uses it.
#[async_trait]
pub trait TaskProvider: Send + Sync {
    /// Start a task under a client-chosen identity. Refused if that identity is already live,
    /// distinguishably from any other refusal (FR-031c).
    async fn start(&self, request: &StartRequest) -> ProviderResult<Pid>;

    /// Reach a task this client did not start in this session, or started and lost (A-TASKLIFE).
    async fn attach(&self, ws: &WorkspaceId, task: &TaskId) -> ProviderResult<AttachResult>;

    /// Every task the engine holds, or every task of one workspace.
    ///
    /// The recovery path for a client that has lost its stored identities entirely: without it
    /// those tasks keep running and are unreachable until the instance idles out (SC-023).
    async fn list(&self, ws: Option<&WorkspaceId>) -> ProviderResult<Vec<TaskSummary>>;

    /// Bytes to the task's input, unchanged. A notification: no response, and no error a caller
    /// can act on.
    async fn write_stdin(&self, task: &TaskId, data: &[u8]) -> ProviderResult<()>;

    /// A new terminal size. Silently ignored by a task started without a terminal.
    async fn resize(&self, task: &TaskId, cols: u16, rows: u16) -> ProviderResult<()>;

    /// The initial signal. `SIGTERM` escalates to `SIGKILL` after the grace period; `SIGINT`
    /// does not escalate (§4.8).
    async fn terminate(&self, task: &TaskId, signal: TerminateSignal) -> ProviderResult<()>;

    /// Finish with a workspace: stops its tasks and releases its watches.
    ///
    /// Deliberately **not** what a dropped connection means. Under A-TASKLIFE a drop leaves
    /// tasks running, because a laptop moving between networks must not kill a build.
    async fn close_workspace(&self, ws: &WorkspaceId) -> ProviderResult<()>;
}

/// The owner every method of the local provider names.
pub const LOCAL_OWNER: Owner = Owner::F015LocalMode;
