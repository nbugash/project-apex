//! Starting a task: everything decided before a process exists.
//!
//! Containment, the refusal of a live identity, the environment merge and the default terminal
//! size all happen here, above the port. That is what lets each of them be tested against
//! `FakeRunner` with no process at all — and, for the refusal, it is what makes FR-031c true
//! rather than merely reported: the second process must never be created, not created and then
//! cleaned up.

use std::collections::BTreeMap;
use std::sync::Arc;

use apex_protocol::wire::{Pid, RunTaskParams, TaskId};

use crate::application::ports::file_system::FileSystem;
use crate::application::ports::roots::{RootError, WorkspaceRoots};
use crate::application::ports::task_runner::{
    ResourceLimits, SpawnFailure, SpawnRequest, SpawnedTask, TaskRunner,
};
use crate::domain::path::{PathRefusal, ResolvedPath};
use crate::domain::task::{EnvOverrides, Shape, StartRefused, Task, TaskSet, TaskState};

/// The terminal size a task gets when the client names neither dimension.
///
/// **In the use case and not the runner.** FR-006b requires a stated quantity fixed in the plan
/// rather than a judgement made per task, and the port must invent no size of its own -- so the
/// decision lives where decisions live and the adapter is handed a number somebody chose. The
/// value is plan.md's *Default terminal size*, and it is specifically not the kernel's 0 x 0,
/// which is a size no display has and the one value `resizePty` refuses.
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// Why a task was not started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartRefusal {
    /// The engine has never been told about this workspace (`-32001`).
    NotRegistered,
    /// Registered, and the root directory is gone (`-32009`).
    RootGone,
    /// `cwd` escapes the workspace root (`-32002`). Returned identically whether or not the
    /// escaped target exists, so a refusal cannot be used to probe the host's filesystem.
    PathRefused,
    /// Inside the root and not there (`-32003`).
    NotFound,
    /// The identity is live (`-32010`). The caller's answer is to attach, not to retry.
    AlreadyRunning,
    /// The command itself could not be started (`-32011`). The developer's mistake to fix.
    CouldNotStart(SpawnFailure),
}

/// Merge the caller's variables **over** the engine's, rather than replacing them.
///
/// Replacement is the more obvious reading of a bare parameter and the wrong default: a task
/// started with one variable set would lose `PATH` and `HOME` and fail for a reason that looks
/// nothing like its cause (§4.8, spec Assumptions).
///
/// Sorted, because the port takes a slice and a test asserting on what a task was spawned with
/// should not depend on hash order.
fn merged_environment(overrides: &EnvOverrides) -> Vec<(String, String)> {
    let mut merged: BTreeMap<String, String> = std::env::vars().collect();
    for (k, v) in overrides.iter() {
        merged.insert(k.clone(), v.clone());
    }
    merged.into_iter().collect()
}

/// The shape a request asks for, with the default applied.
fn shape_of(params: &RunTaskParams) -> Shape {
    if params.pty {
        Shape::Pty {
            cols: params.cols.unwrap_or(DEFAULT_COLS),
            rows: params.rows.unwrap_or(DEFAULT_ROWS),
        }
    } else {
        Shape::Pipes
    }
}

/// Start a task, or say why not.
///
/// The order matters and is the requirement rather than an implementation detail: the identity
/// is checked **before** anything is spawned, so a client racing its own reconnection cannot
/// start a second build and then be told it should not have (FR-031c, SC-022).
pub fn start_task(
    params: &RunTaskParams,
    roots: &dyn WorkspaceRoots,
    fs: &dyn FileSystem,
    runner: &dyn TaskRunner,
    tasks: &mut TaskSet,
) -> Result<(Pid, SpawnedTask), StartRefusal> {
    if tasks.contains(&params.task_id) {
        return Err(StartRefusal::AlreadyRunning);
    }

    let root = roots
        .resolve(params.workspace_id.0.as_str())
        .map_err(|e| match e {
            RootError::NotRegistered | RootError::Conflict { .. } => StartRefusal::NotRegistered,
            RootError::Gone | RootError::Unusable => StartRefusal::RootGone,
        })?;

    // Untrusted, like every path off the wire. `ResolvedPath` has no other constructor, so the
    // port cannot be handed an uncontained directory however this function changes later.
    let cwd = ResolvedPath::resolve(&root, params.cwd.as_deref().unwrap_or("."), fs).map_err(
        |e| match e {
            PathRefusal::Refused => StartRefusal::PathRefused,
            PathRefusal::NotFound => StartRefusal::NotFound,
        },
    )?;

    let env = EnvOverrides::new(params.env.clone().unwrap_or_default());
    let merged = merged_environment(&env);
    let shape = shape_of(params);

    let spawned = runner
        .spawn(&SpawnRequest {
            command: &params.command,
            cwd: &cwd,
            env: &merged,
            shape,
            limits: ResourceLimits::FIXED,
        })
        .map_err(StartRefusal::CouldNotStart)?;

    let task = Task {
        id: params.task_id.clone(),
        workspace: params.workspace_id.clone(),
        command: params.command.clone(),
        shape,
        pid: spawned.pid,
        env,
        state: TaskState::Running,
    };
    // Cannot fail: the identity was checked above and nothing else holds the set meanwhile.
    // Mapped rather than unwrapped so a future caller that drops the check gets a refusal
    // instead of a panic.
    tasks
        .start(task)
        .map_err(|StartRefused::AlreadyRunning| StartRefusal::AlreadyRunning)?;

    Ok((spawned.pid, spawned))
}

/// The handles a caller keeps after a successful start.
pub struct StartedTask {
    pub pid: Pid,
    pub control: Arc<dyn crate::application::ports::task_runner::TaskControl>,
    pub output: Box<dyn crate::application::ports::task_runner::TaskOutput>,
}

/// Split a spawn into the halves their owners keep, so a caller never holds both.
pub fn split(id: &TaskId, spawned: SpawnedTask) -> (TaskId, StartedTask) {
    (
        id.clone(),
        StartedTask {
            pid: spawned.pid,
            control: spawned.control,
            output: spawned.output,
        },
    )
}
