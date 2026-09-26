//! The commands the terminal panel calls.
//!
//! Input arriving here is untrusted regardless of what the interface layer already checked
//! (Constitution Principle VI), so every argument is validated in the core and rejected rather
//! than coerced. The webview is not a trusted caller: it runs remote-authored content in the
//! general case, and a task identity or a signal name reaching a syscall unchecked is the exact
//! shape §4.7 is written against.
//!
//! # Why `run` is a request and `write_stdin` is not
//!
//! `execution/writeStdin` and `execution/resizePty` are notifications (§4.2): no id, no reply,
//! nothing to await. That is a deliberate property of keystroke-rate traffic -- a reply per
//! keypress doubles the traffic of typing and tells the typist nothing the echo will not -- and
//! the cost is that an unknown task, an exited task and a full buffer are indistinguishable
//! here. Anything that must be refused visibly stays a request, which is why `run` and
//! `terminate` return errors and these two cannot.

use std::sync::Arc;

use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use tauri::State;

use crate::application::error::ShellError;
use crate::application::ports::task_provider::{TaskProvider, TerminateSignal};

/// What the panel needs back from a start: enough to address the task, and nothing else.
#[derive(serde::Serialize)]
pub struct Started {
    pub task_id: String,
    pub pid: i32,
}

pub struct Tasks {
    /// Absent when no engine is configured. `None` rather than a refusing implementation,
    /// because the commands must answer "there is no engine" distinguishably from "the engine
    /// refused you" -- the first is a configuration the developer can fix.
    pub provider: Option<Arc<dyn TaskProvider>>,
    /// The same connection, for the one call that is not a task.
    ///
    /// A task's `workspaceId` has to name a workspace the **engine** has registered, and until
    /// now nothing on this side ever sent `workspace/register` -- F003 built the provider and
    /// no caller. Registration happens when a workspace is opened rather than lazily at the
    /// first task, because that is the moment the root path is in hand and the moment the
    /// client itself registers: the two learning about a workspace together is one fact, and
    /// two lazy paths that must agree is two.
    pub sender: Option<Arc<dyn crate::application::ports::request_sender::RequestSender>>,
    /// The workspace the engine currently knows about, set when one is registered.
    ///
    /// Held here rather than passed in by the webview, and that is a Principle VI decision
    /// rather than a convenience: a `workspaceId` accepted from the interface is a caller
    /// naming which workspace a command runs in, and the interface has no business choosing
    /// that. The core registered it, so the core knows it.
    pub current: std::sync::Mutex<Option<String>>,
}

/// Tell the engine about a workspace. Best effort, and says so.
///
/// A failure here is reported and not fatal: the client's own registration has already
/// succeeded, the cached tree is still usable, and the only thing lost is the ability to start
/// a task in it -- which the task command reports on its own terms when it is tried.
pub async fn register_with_engine(
    sender: &Arc<dyn crate::application::ports::request_sender::RequestSender>,
    workspace_id: &str,
    path: &str,
) {
    let params = serde_json::json!({ "workspace_id": workspace_id, "path": path });
    let outcome = sender
        .send(crate::application::ports::transport::Request::interactive(
            "workspace/register",
            params.to_string(),
        ))
        .await;
    if !matches!(outcome, crate::domain::request::RequestOutcome::Answered(_)) {
        crate::logging::warn(&format!(
            "the engine did not register {workspace_id}: {outcome:?}"
        ));
    }
}

/// The largest input a single call may carry.
///
/// A keystroke is a handful of bytes and a paste is bounded by what a person pastes; this cap
/// exists so a webview that has been taken over cannot use `writeStdin` as an amplifier against
/// a 1 MiB frame limit it does not otherwise touch. 64 KiB matches the engine's own chunk size,
/// so a legitimate paste never straddles it by accident.
const MAX_STDIN_BYTES: usize = 64 * 1024;

fn provider(tasks: &State<'_, Tasks>) -> Result<Arc<dyn TaskProvider>, ShellError> {
    tasks.provider.clone().ok_or(ShellError::NotConnected)
}

/// Reject an identity that is empty or implausibly long before it reaches the wire.
///
/// The engine validates too (Principle VI puts the check on both sides). This one exists so a
/// malformed identity is refused where the caller can see it, rather than becoming a `-32602`
/// that reads like an engine fault.
fn parse_task_id(raw: &str) -> Result<TaskId, ShellError> {
    if raw.is_empty() || raw.len() > 256 {
        return Err(ShellError::InvalidRegion);
    }
    Ok(TaskId(raw.to_string()))
}

#[tauri::command]
pub async fn task_run(
    task_id: String,
    command: Vec<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    tasks: State<'_, Tasks>,
) -> Result<Started, ShellError> {
    let id = parse_task_id(&task_id)?;
    // An empty argv would reach `execve` as a null command. Refused here because the failure it
    // otherwise produces names the engine rather than the caller.
    if command.is_empty() {
        return Err(ShellError::InvalidRegion);
    }
    let workspace_id = tasks
        .current
        .lock()
        .expect("current workspace")
        .clone()
        .ok_or(ShellError::NotConnected)?;
    let request = RunTaskParams {
        workspace_id: WorkspaceId(workspace_id),
        task_id: id.clone(),
        command,
        cwd: None,
        env: None,
        // Always a terminal. This command exists to serve the terminal panel, and a task
        // started without one would merge nothing, echo nothing and answer no ctrl-C.
        pty: true,
        cols,
        rows,
    };
    let pid = provider(&tasks)?.start(&request).await.map_err(|e| {
        crate::logging::warn(&format!("task {} did not start: {e:?}", id.0));
        ShellError::PersistenceFailed
    })?;
    Ok(Started {
        task_id: id.0,
        pid: pid.0,
    })
}

#[tauri::command]
pub async fn task_write_stdin(
    task_id: String,
    data: String,
    tasks: State<'_, Tasks>,
) -> Result<(), ShellError> {
    let id = parse_task_id(&task_id)?;
    let bytes = apex_protocol::base64::decode(&data).map_err(|_| ShellError::InvalidRegion)?;
    if bytes.len() > MAX_STDIN_BYTES {
        return Err(ShellError::InvalidRegion);
    }
    provider(&tasks)?
        .write_stdin(&id, &bytes)
        .await
        .map_err(|_| ShellError::PersistenceFailed)
}

#[tauri::command]
pub async fn task_resize(
    task_id: String,
    cols: u16,
    rows: u16,
    tasks: State<'_, Tasks>,
) -> Result<(), ShellError> {
    let id = parse_task_id(&task_id)?;
    // 0 x 0 is the kernel's default and the one size `resizePty` refuses (§4.8). Refused here
    // too, so a panel that measures itself before layout does not send one.
    if cols == 0 || rows == 0 {
        return Err(ShellError::InvalidRegion);
    }
    provider(&tasks)?
        .resize(&id, cols, rows)
        .await
        .map_err(|_| ShellError::PersistenceFailed)
}

#[tauri::command]
pub async fn task_terminate(
    task_id: String,
    signal: String,
    tasks: State<'_, Tasks>,
) -> Result<(), ShellError> {
    let id = parse_task_id(&task_id)?;
    // Parsed against the three §4.8 admits rather than passed through. A signal name reaching a
    // syscall from the webview unchecked is the shape Principle VI exists to refuse.
    let signal = match signal.as_str() {
        "SIGINT" => TerminateSignal::Int,
        "SIGTERM" => TerminateSignal::Term,
        "SIGKILL" => TerminateSignal::Kill,
        _ => return Err(ShellError::InvalidRegion),
    };
    provider(&tasks)?
        .terminate(&id, signal)
        .await
        .map_err(|_| ShellError::PersistenceFailed)
}
