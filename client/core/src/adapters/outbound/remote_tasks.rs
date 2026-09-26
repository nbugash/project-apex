//! The task provider over the transport: §4.8's execution calls, as frames.
//!
//! An outbound adapter and nothing more. Every rule about what a task is, when one may start and
//! what an ending means lives above this line; what is here is composing a frame, sending it, and
//! turning §4.4's codes into typed errors so that no caller ever parses a message.
//!
//! **Two kinds of call, and the difference is not cosmetic.** `runTask`, `attach`, `list`,
//! `terminate` and `close` are requests and have answers. `writeStdin` and `resizePty` are
//! notifications: §4.2 gives them no response, so an unknown task, an exited task and a full
//! buffer are all indistinguishable here. That is the contract rather than a gap -- there is no
//! response to carry a refusal in -- and it is why they go through `notify` rather than `send`,
//! which would wait for a reply that never arrives.

use async_trait::async_trait;
use std::sync::Arc;

use crate::application::ports::request_sender::RequestSender;
use crate::application::ports::task_provider::{
    AttachResult, Pid, StartRequest, TaskId, TaskProvider, TaskSummary, TerminateSignal,
};
use crate::application::ports::transport::Request;
use crate::application::ports::workspace_provider::{ProviderError, ProviderResult};
use crate::domain::request::RequestOutcome;
use crate::domain::workspace::WorkspaceId;
use apex_protocol::wire::{
    codes, ListParams, ResizePtyParams, TerminateParams, WorkspaceCloseParams, WriteStdinParams,
};

pub struct RemoteTasks {
    transport: Arc<dyn RequestSender>,
}

impl RemoteTasks {
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
                // A malformed result is an error, never a panic and never a default: what the
                // engine sends is untrusted at this end too (Principle VI).
                serde_json::from_value(result)
                    .map_err(|e| ProviderError::Transport(format!("result of {method}: {e}")))
            }
            RequestOutcome::Failed { code, message } => Err(map_code(code, message)),
            RequestOutcome::TimedOut => Err(ProviderError::Transport("timed out".into())),
            RequestOutcome::Withdrawn => Err(ProviderError::Transport("withdrawn".into())),
            RequestOutcome::ConnectionLost => Err(ProviderError::Offline),
        }
    }

    /// Send a notification. Encoding is the only thing that can fail, and it cannot fail for the
    /// types this is used with -- but it is reported rather than unwrapped, because a panic in
    /// the client over a keystroke is a worse outcome than a dropped keystroke.
    async fn send_notification<P: serde::Serialize>(
        &self,
        method: &str,
        params: &P,
    ) -> ProviderResult<()> {
        let body = serde_json::to_string(params)
            .map_err(|e| ProviderError::Transport(format!("encoding {method}: {e}")))?;
        self.transport
            .notify(Request::interactive(method, body))
            .await;
        Ok(())
    }
}

/// §4.4's codes, as the typed errors a caller branches on.
///
/// The three task codes are kept apart deliberately. `-32011` is never reported as `NotFound`,
/// which §4.4 reserves for a path inside a workspace: a missing executable and a missing file
/// lead to different things being said to the developer.
fn map_code(code: i32, message: String) -> ProviderError {
    match code {
        codes::TASK_NOT_FOUND => ProviderError::TaskNotFound,
        codes::TASK_ALREADY_RUNNING => ProviderError::TaskAlreadyRunning,
        codes::COMMAND_NOT_STARTED => ProviderError::CommandNotStarted { reason: message },
        codes::WORKSPACE_NOT_REGISTERED => ProviderError::UnknownWorkspace,
        codes::WORKSPACE_GONE => ProviderError::WorkspaceGone,
        codes::PATH_REFUSED => ProviderError::Refused,
        codes::NOT_FOUND => ProviderError::NotFound,
        _ => ProviderError::Transport(message),
    }
}

#[async_trait]
impl TaskProvider for RemoteTasks {
    async fn start(&self, request: &StartRequest) -> ProviderResult<Pid> {
        #[derive(serde::Deserialize)]
        struct Started {
            pid: Pid,
        }
        // The request is the wire type unchanged, because the client composes the frame and
        // nothing here reshapes it -- which is what `StartRequest = RunTaskParams` already says.
        let started: Started = self.call("execution/runTask", request).await?;
        Ok(started.pid)
    }

    async fn attach(&self, ws: &WorkspaceId, task: &TaskId) -> ProviderResult<AttachResult> {
        #[derive(serde::Serialize)]
        struct Params<'a> {
            workspace_id: &'a str,
            task_id: &'a TaskId,
        }
        self.call(
            "execution/attach",
            &Params {
                workspace_id: &ws.0,
                task_id: task,
            },
        )
        .await
    }

    async fn list(&self, ws: Option<&WorkspaceId>) -> ProviderResult<Vec<TaskSummary>> {
        #[derive(serde::Deserialize)]
        struct Listed {
            tasks: Vec<TaskSummary>,
        }
        let params = ListParams {
            // The client's `WorkspaceId` **is** the wire type, re-exported. Rebuilding one from
            // its own string would be a conversion between a type and itself.
            workspace_id: ws.cloned(),
        };
        let listed: Listed = self.call("execution/list", &params).await?;
        Ok(listed.tasks)
    }

    async fn write_stdin(&self, task: &TaskId, data: &[u8]) -> ProviderResult<()> {
        // Encoded here and nowhere else, from bytes that were never a `String`. A keystroke
        // carrying a control byte or a lone high byte has to survive this unchanged.
        let params = WriteStdinParams {
            task_id: task.clone(),
            data: apex_protocol::base64::encode(data),
        };
        self.send_notification("execution/writeStdin", &params)
            .await
    }

    async fn resize(&self, task: &TaskId, cols: u16, rows: u16) -> ProviderResult<()> {
        let params = ResizePtyParams {
            task_id: task.clone(),
            cols,
            rows,
        };
        self.send_notification("execution/resizePty", &params).await
    }

    async fn terminate(&self, task: &TaskId, signal: TerminateSignal) -> ProviderResult<()> {
        let params = TerminateParams {
            task_id: task.clone(),
            signal,
        };
        // A request, not a notification: `-32006` for a released identity is an answer a caller
        // acts on, and FR-019's success for an already-exited task is equally an answer.
        let _: serde_json::Value = self.call("execution/terminate", &params).await?;
        Ok(())
    }

    async fn close_workspace(&self, ws: &WorkspaceId) -> ProviderResult<()> {
        let params = WorkspaceCloseParams {
            workspace_id: ws.clone(),
        };
        let _: serde_json::Value = self.call("workspace/close", &params).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_task_codes_stay_apart() {
        // A client choosing its own task ids needs "you already have one of these" to be
        // distinguishable from "you have none". Collapsing either into the other makes a correct
        // client look broken in one direction and hides a real conflict in the other.
        assert!(matches!(
            map_code(codes::TASK_NOT_FOUND, String::new()),
            ProviderError::TaskNotFound
        ));
        assert!(matches!(
            map_code(codes::TASK_ALREADY_RUNNING, String::new()),
            ProviderError::TaskAlreadyRunning
        ));
        assert!(matches!(
            map_code(codes::COMMAND_NOT_STARTED, "no such file".into()),
            ProviderError::CommandNotStarted { .. }
        ));
    }

    #[test]
    fn an_unstartable_command_is_never_reported_as_a_missing_file() {
        // §4.4 reserves -32003 for a path inside a workspace. A missing executable and a missing
        // source file lead to different things being said to the developer, and a client that
        // conflates them says the wrong one.
        let unstartable = map_code(codes::COMMAND_NOT_STARTED, "no such file".into());
        assert!(!matches!(unstartable, ProviderError::NotFound));
        assert!(matches!(
            map_code(codes::NOT_FOUND, String::new()),
            ProviderError::NotFound
        ));
    }

    #[test]
    fn the_reason_the_engine_gave_survives() {
        // -32011 carries `data.reason`, and dropping it leaves the developer with "could not
        // start" and no way to tell a typo from a permission problem.
        let ProviderError::CommandNotStarted { reason } =
            map_code(codes::COMMAND_NOT_STARTED, "permission denied".into())
        else {
            panic!("expected CommandNotStarted");
        };
        assert_eq!(reason, "permission denied");
    }

    #[test]
    fn an_unrecognised_code_is_transport_rather_than_a_guess() {
        // Principle VI: what the engine sends is untrusted. A code this client does not know is
        // reported as what it is, not mapped to the nearest familiar variant.
        assert!(matches!(
            map_code(-31999, "something new".into()),
            ProviderError::Transport(_)
        ));
    }
}
