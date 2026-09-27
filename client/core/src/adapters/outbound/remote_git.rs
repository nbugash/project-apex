//! Outbound adapter: `GitProvider` over the JSON-RPC transport.
//!
//! One provider call, at most one protocol request. Paging is the caller's loop and not this
//! adapter's: an adapter that quietly pulled every page would make a single call unbounded in
//! time and in memory, on exactly the repositories where that matters most.

use crate::application::ports::git_provider::GitProvider;
use crate::application::ports::request_sender::RequestSender;
use crate::application::ports::transport::Request;
use crate::application::ports::workspace_provider::{ProviderError, ProviderResult};
use crate::domain::request::RequestOutcome;
use crate::domain::workspace::WorkspaceId;
use apex_protocol::wire::{self, codes};
use async_trait::async_trait;
use std::sync::Arc;

pub struct RemoteGitProvider {
    transport: Arc<dyn RequestSender>,
}

impl RemoteGitProvider {
    pub fn new(transport: Arc<dyn RequestSender>) -> Self {
        Self { transport }
    }

    async fn call<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
    ) -> ProviderResult<R> {
        let body = serde_json::to_string(params)
            .map_err(|e| ProviderError::Transport(format!("encoding {method}: {e}")))?;
        match self
            .transport
            .send(Request::interactive(method, body))
            .await
        {
            RequestOutcome::Answered(json) => {
                let value: serde_json::Value = serde_json::from_str(&json)
                    .map_err(|e| ProviderError::Transport(format!("reply to {method}: {e}")))?;
                let result = value.get("result").cloned().unwrap_or(value);
                // What the engine sends is untrusted (Principle VI): a malformed result is an
                // error, never a default value that would read as "nothing has changed".
                serde_json::from_value(result)
                    .map_err(|e| ProviderError::Transport(format!("result of {method}: {e}")))
            }
            RequestOutcome::Failed { code, message } => Err(map_code(code, message)),
            RequestOutcome::TimedOut => Err(ProviderError::Transport("timed out".into())),
            RequestOutcome::Withdrawn => Err(ProviderError::Transport("withdrawn".into())),
            RequestOutcome::ConnectionLost => Err(ProviderError::Offline),
        }
    }
}

#[async_trait]
impl GitProvider for RemoteGitProvider {
    async fn status(
        &self,
        workspace: &WorkspaceId,
        cursor: Option<&str>,
    ) -> ProviderResult<wire::GitStatusResult> {
        self.call(
            "git/getStatus",
            &wire::GitStatusParams {
                workspace_id: wire::WorkspaceId(workspace.0.clone()),
                cursor: cursor.map(str::to_owned),
                // Left to the engine. §4.1's cap is the engine's constraint and naming a number
                // here would be a second place it is written (Principle II).
                limit: None,
            },
        )
        .await
    }

    async fn file_diff(
        &self,
        workspace: &WorkspaceId,
        relative_path: &str,
    ) -> ProviderResult<wire::GitDiffResult> {
        self.call(
            "git/getFileDiff",
            &wire::GitDiffParams {
                workspace_id: wire::WorkspaceId(workspace.0.clone()),
                relative_path: relative_path.to_owned(),
            },
        )
        .await
    }
}

/// §4.4's codes as typed errors, so no caller parses a message.
///
/// A stale cursor arrives as invalid params, which no §4.4 code claims, so it falls to
/// `Transport`. That is the right outcome and not a gap: the accumulator discards a pull on
/// **any** error and keeps the state it already had. What must never happen is a refused cursor
/// read as "start again", which would assemble one picture out of two snapshots -- and there is
/// no code path here that could, because starting again is `cursor: None` and only the caller
/// decides that (contracts/git-status.md).
fn map_code(code: i32, message: String) -> ProviderError {
    match code {
        codes::WORKSPACE_NOT_REGISTERED => ProviderError::UnknownWorkspace,
        codes::WORKSPACE_GONE => ProviderError::WorkspaceGone,
        codes::PATH_REFUSED => ProviderError::Refused,
        codes::NOT_FOUND => ProviderError::NotFound,
        _ => ProviderError::Transport(format!("{code}: {message}")),
    }
}
