//! Starting a task: everything decided before a process exists.
//!
//! Containment, the refusal of a live identity, the environment merge and the default terminal
//! size all happen here, above the port. That is what lets each of them be tested against
//! `FakeRunner` with no process at all — and, for the refusal, it is what makes FR-031c true
//! rather than merely reported: the second process must never be created, not created and then
//! cleaned up.

use std::collections::BTreeMap;
use std::sync::Arc;

use apex_protocol::wire::{
    AttachResult, Pid, RunTaskParams, SignalName, TaskId, TaskSummary, WorkspaceId,
};

use crate::application::ports::clock::Millis;
use crate::application::ports::file_system::FileSystem;
use crate::application::ports::roots::{RootError, WorkspaceRoots};
use crate::application::ports::task_runner::{
    ResourceLimits, SpawnFailure, SpawnRequest, SpawnedTask, TaskControl, TaskRunner,
};
use crate::domain::path::{PathRefusal, ResolvedPath};
use crate::domain::task::{
    EnvOverrides, ExitStatus, Shape, StartRefused, Task, TaskSet, TaskSignal, TaskState,
};

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
    merge_over(std::env::vars(), overrides)
}

/// The merge itself, with the inherited environment supplied rather than read.
///
/// Pure, so the precedence rule can be tested without mutating this process's environment --
/// which is global state that sibling tests fork through, and therefore a race rather than a
/// test. Reading the real environment is the caller's line above, which is where an adapter
/// concern belongs (Principle VIII).
fn merge_over(
    inherited: impl Iterator<Item = (String, String)>,
    overrides: &EnvOverrides,
) -> Vec<(String, String)> {
    let mut merged: BTreeMap<String, String> = inherited.collect();
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

/// How long a `SIGTERM` is given before `SIGKILL` follows (§4.8, plan.md *Escalation grace*).
///
/// Stated here, in the use case, because it is a decision rather than a mechanism: the adapter
/// is handed a deadline somebody chose, and the same number is what a test asserts against
/// instead of restating it.
pub const ESCALATION_GRACE_MS: Millis = 5_000;

/// Write bytes to a task's input.
///
/// **Returns nothing, by construction.** `execution/writeStdin` is a notification and §4.2 gives
/// it no response, so an unknown id, an exited task and a full buffer are indistinguishable to
/// the caller. Making this fallible would invent a failure the protocol has nowhere to put.
///
/// The outcome is still distinguished *here*, so a caller can tell "nobody to write to" from
/// "the write was refused" even though neither reaches the client. It takes no task id: the
/// caller resolved the control from one already, and a parameter carried only to be logged by
/// nothing is dead weight.
pub fn write_input(data: &[u8], control: Option<Arc<dyn TaskControl>>) -> InputOutcome {
    let Some(control) = control else {
        return InputOutcome::NoSuchTask;
    };
    if data.is_empty() {
        // A legal frame that asks for nothing. Writing zero bytes to a pipe is not the same as
        // writing nothing -- on some paths it signals end of file -- so it is skipped rather
        // than passed through.
        return InputOutcome::Written;
    }
    match control.write_stdin(data) {
        Ok(()) => InputOutcome::Written,
        Err(_) => InputOutcome::Refused,
    }
}

/// What a write did. Never reaches the client; exists so a test and a log can tell these apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    Written,
    NoSuchTask,
    Refused,
}

/// Change a task's terminal size.
///
/// A no-op for a task started without a terminal, and for a zero in either dimension. Zero is
/// the value some programs read as "no terminal at all" and change what they print accordingly,
/// so forwarding one would alter a task's behaviour rather than its layout -- and the client
/// that sent it is almost certainly reporting an element it has not laid out yet.
///
/// Returns nothing to the client, for `write_input`'s reason.
pub fn resize_task(
    shape: Option<Shape>,
    cols: u16,
    rows: u16,
    control: Option<Arc<dyn TaskControl>>,
) -> ResizeOutcome {
    let Some(shape) = shape else {
        return ResizeOutcome::NoSuchTask;
    };
    if !matches!(shape, Shape::Pty { .. }) {
        // A task with pipes has no window to resize. Not an error: a client showing a panel for
        // it is right to report its size, and A-TASKSTREAM makes the shape the client's choice.
        return ResizeOutcome::NoTerminal;
    }
    if cols == 0 || rows == 0 {
        return ResizeOutcome::Refused;
    }
    let Some(control) = control else {
        return ResizeOutcome::NoSuchTask;
    };
    match control.resize(cols, rows) {
        Ok(()) => ResizeOutcome::Resized,
        Err(_) => ResizeOutcome::Refused,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeOutcome {
    Resized,
    NoTerminal,
    NoSuchTask,
    Refused,
}

/// Stop a task: send exactly the named signal, to the group, and decide what follows.
///
/// **This never sleeps.** A `Term` registers a deadline and returns; the waiting and the second
/// signal belong to the escalation thread. A use case that slept here would block the dispatch
/// thread for five seconds per stop -- and that thread is also the only reader of the client's
/// stdin, so nothing would be read off the pipe in the meantime, which is §1.4 and FR-012
/// failing through the mechanism meant to satisfy FR-024.
///
/// The deadline is keyed on the `(TaskId, Pid)` pair rather than on the id alone. An id can be
/// reused once its task has ended, and a deadline that found its target by identity could kill
/// a process that merely inherited the name.
pub fn stop_task(id: &TaskId, signal: TaskSignal, now: Millis) -> Result<StopPlan, StopRefusal> {
    Ok(StopPlan {
        send: signal,
        // `Int` deliberately does not escalate: a program that legitimately handles an interrupt
        // must not be killed for having handled it. `Kill` has nothing to escalate to.
        escalate_at: match signal {
            TaskSignal::Term => Some(now + ESCALATION_GRACE_MS),
            TaskSignal::Int | TaskSignal::Kill => None,
        },
        id: id.clone(),
    })
}

/// What a stop asks the adapter to do. A value rather than an effect, so the policy is testable
/// without a process, a clock thread or a signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPlan {
    pub id: TaskId,
    /// Sent first, and to the **group**: a build's own children are the things still holding the
    /// terminal, and signalling only the named process leaves them running (FR-006a, SC-027).
    pub send: TaskSignal,
    /// When `SIGKILL` follows, if it does.
    pub escalate_at: Option<Millis>,
}

/// Why a stop could not be planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRefusal {
    /// No live identity. `-32006`; see FR-019 for why an already-exited task is not this.
    NotFound,
}

/// Closing a workspace: what to stop, and when to answer.
///
/// **The response is written once every task has been signalled, not once the last has ended**
/// (A-WSCLOSE, as amended). Ending takes up to the five-second escalation, and this runs on the
/// single dispatch thread, which is also the only reader of the client's stdin -- waiting there
/// would mean five seconds in which no keystroke, resize or cancellation is so much as read off
/// the pipe. SC-013 stays checkable without it: "zero of its tasks are running" is observed
/// through each task's `onExit`, which is a defined event in a defined order, so a test waits for
/// N exits rather than for a sleep.
///
/// One deadline per task, registered together, so the escalations **overlap**: closing ten
/// workspaces' worth of tasks costs about five seconds, not fifty.
pub fn close_workspace(workspace: &WorkspaceId, now: Millis, tasks: &mut TaskSet) -> Vec<StopPlan> {
    // Drained, not merely listed. Draining twice yields nothing the second time, which is what
    // lets a second `workspace/close` be answered as `-32001` rather than repeating the first --
    // a workspace never closes itself, so a second close means the client has lost track of its
    // own state and telling it so is a service (A-WSCLOSE).
    tasks
        .drain_for_workspace(workspace)
        .into_iter()
        .map(|task| StopPlan {
            id: task.id,
            // `Term` and not `Kill`: closing a workspace is a developer finishing with it, not an
            // emergency, and a build given no chance to remove its half-written output leaves the
            // next one to discover it.
            send: TaskSignal::Term,
            escalate_at: Some(now + ESCALATION_GRACE_MS),
        })
        .collect()
}

/// Why an attach could not be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachRefusal {
    /// The workspace does not own this task, or does not exist (`-32001`).
    ///
    /// **Not `-32006`.** A task the engine holds but a different workspace owns is not a missing
    /// task; telling a client it has no such task when another workspace does would send it
    /// looking for a bug it does not have.
    NotThisWorkspace,
    /// No live identity (`-32006`).
    NotFound,
}

/// Reach a task this client did not start in this session, or started and lost (A-TASKLIFE).
///
/// A **pure read**: it resolves the identity and reports what the engine already knows.
/// Attaching deliberately has no side effect on the process -- it does not resize it, signal it,
/// or change what it is doing -- which is attach guarantee 9 and the reason a client has to send
/// its own size afterwards.
///
/// `retained` is a **byte count**, not the bytes. The bytes follow as ordinary `onStdout`
/// notifications after the response, so a reattaching client's replay arrives on the path it
/// already handles rather than as a second, larger shape inside a reply.
pub fn attach_task(
    workspace: &WorkspaceId,
    id: &TaskId,
    retained: usize,
    tasks: &TaskSet,
) -> Result<AttachResult, AttachRefusal> {
    let task = tasks.get(id).ok_or(AttachRefusal::NotFound)?;
    if &task.workspace != workspace {
        return Err(AttachRefusal::NotThisWorkspace);
    }
    let (running, exit_code, signal) = match task.state {
        TaskState::Running => (true, None, None),
        // FR-031b and SC-020: `running: false` alone says only that it is over. A client that
        // reattaches to a finished build needs to know **how** it finished, and the same one-of-two
        // shape §9 uses everywhere else says it.
        TaskState::Ended(ExitStatus::Exited { code }) => (false, Some(code), None),
        TaskState::Ended(ExitStatus::Signalled { signal }) => {
            (false, None, Some(SignalName::from_number(signal)))
        }
    };
    Ok(AttachResult {
        pid: task.pid,
        running,
        retained: retained as u64,
        exit_code,
        signal,
    })
}

/// Every task the engine holds, or every task of one workspace.
///
/// A **pure read over `TaskSet` that touches the port zero times**. Asking each task's control
/// whether it is still running would turn a listing into N syscalls, and the answer would still
/// be the engine's own record a moment later -- the record is what `onExit` updates, and it is
/// the same thing a client would be told.
///
/// The recovery path for a client that has lost its stored identities entirely: without it those
/// tasks keep running and are unreachable until the instance idles out (SC-023).
pub fn list_tasks(workspace: Option<&WorkspaceId>, tasks: &TaskSet) -> Vec<TaskSummary> {
    tasks
        .list(workspace)
        .into_iter()
        .map(|task| {
            let (running, exit_code, signal) = match task.state {
                TaskState::Running => (true, None, None),
                TaskState::Ended(ExitStatus::Exited { code }) => (false, Some(code), None),
                TaskState::Ended(ExitStatus::Signalled { signal }) => {
                    (false, None, Some(SignalName::from_number(signal)))
                }
            };
            TaskSummary {
                task_id: task.id.clone(),
                workspace_id: task.workspace.clone(),
                // `command` and **never** `env`. FR-005a keeps a task's environment out of
                // anything that can be read back, and a listing is exactly that.
                command: task.command.clone(),
                pty: matches!(task.shape, Shape::Pty { .. }),
                pid: task.pid,
                running,
                exit_code,
                signal,
            }
        })
        .collect()
}

/// Stop every task the engine holds, whatever workspace owns it.
///
/// A-TASKEXEC: re-executing the engine terminates its tasks, because `exec` replaces the process
/// image and the reader threads go with it -- a task left running would be a process nobody is
/// reading and nobody can reach. The same `Term`-then-`Kill` escalation as `workspace/close`, and
/// for the same reason: a build given no chance to remove its half-written output leaves the next
/// one to discover it.
///
/// The ids are what the new image reports as `unpreserved`, which is how a restart is announced
/// rather than inferred.
pub fn drain_all_tasks(now: Millis, tasks: &mut TaskSet) -> Vec<StopPlan> {
    tasks
        .drain_all()
        .into_iter()
        .map(|task| StopPlan {
            id: task.id,
            send: TaskSignal::Term,
            escalate_at: Some(now + ESCALATION_GRACE_MS),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inherited(pairs: &[(&str, &str)]) -> std::vec::IntoIter<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn overrides(pairs: &[(&str, &str)]) -> EnvOverrides {
        EnvOverrides::new(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        )
    }

    fn value<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn task_in(ws: &str, id: &str) -> Task {
        Task {
            id: TaskId(id.into()),
            workspace: WorkspaceId(ws.into()),
            command: vec!["cargo".into()],
            shape: Shape::Pipes,
            pid: Pid(1),
            env: EnvOverrides::new(BTreeMap::new()),
            state: TaskState::Running,
        }
    }

    #[test]
    fn closing_a_workspace_stops_its_tasks_and_only_its_tasks() {
        // The assertion that matters is the **zero**: a close that signalled everything would
        // satisfy every positive claim about A's tasks stopping, and would end a build in a
        // window the developer is still working in.
        let mut tasks = TaskSet::new();
        for id in ["a1", "a2", "a3"] {
            tasks.start(task_in("A", id)).expect("start");
        }
        tasks.start(task_in("B", "b1")).expect("start");

        let plans = close_workspace(&WorkspaceId("A".into()), 1_000, &mut tasks);

        let mut stopped: Vec<String> = plans.iter().map(|p| p.id.0.clone()).collect();
        stopped.sort();
        assert_eq!(stopped, vec!["a1", "a2", "a3"]);
        assert_eq!(
            plans.iter().filter(|p| p.id.0 == "b1").count(),
            0,
            "closing A produced a plan for a task of B"
        );
        assert!(
            tasks.contains(&TaskId("b1".into())),
            "closing A released a task of B"
        );
    }

    #[test]
    fn every_task_is_stopped_exactly_once() {
        // Twice is not harmless: the second signal lands after the first has been acted on, and
        // between them the pid may have been reused.
        let mut tasks = TaskSet::new();
        for id in ["a1", "a2"] {
            tasks.start(task_in("A", id)).expect("start");
        }
        let plans = close_workspace(&WorkspaceId("A".into()), 0, &mut tasks);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans.iter().filter(|p| p.id.0 == "a1").count(), 1);
    }

    #[test]
    fn the_escalations_overlap_rather_than_queue() {
        // One deadline per task, all at the same instant. Staggering them would make closing ten
        // tasks cost fifty seconds instead of five, and the developer is waiting.
        let mut tasks = TaskSet::new();
        for id in ["a1", "a2", "a3"] {
            tasks.start(task_in("A", id)).expect("start");
        }
        let plans = close_workspace(&WorkspaceId("A".into()), 1_000, &mut tasks);
        let deadlines: Vec<Option<Millis>> = plans.iter().map(|p| p.escalate_at).collect();
        assert_eq!(deadlines.len(), 3);
        assert!(
            deadlines
                .iter()
                .all(|d| *d == Some(1_000 + ESCALATION_GRACE_MS)),
            "the deadlines were staggered: {deadlines:?}"
        );
    }

    #[test]
    fn a_close_sends_term_rather_than_kill() {
        // Closing a workspace is a developer finishing with it, not an emergency. A build given
        // no chance to remove its half-written output leaves the next one to discover it.
        let mut tasks = TaskSet::new();
        tasks.start(task_in("A", "a1")).expect("start");
        let plans = close_workspace(&WorkspaceId("A".into()), 0, &mut tasks);
        assert_eq!(plans[0].send, TaskSignal::Term);
    }

    #[test]
    fn closing_twice_yields_nothing_the_second_time() {
        // What makes a second `workspace/close` answerable as -32001 rather than a silent repeat.
        let mut tasks = TaskSet::new();
        tasks.start(task_in("A", "a1")).expect("start");
        assert_eq!(
            close_workspace(&WorkspaceId("A".into()), 0, &mut tasks).len(),
            1
        );
        assert_eq!(
            close_workspace(&WorkspaceId("A".into()), 0, &mut tasks).len(),
            0
        );
    }

    #[test]
    fn closing_a_workspace_with_no_tasks_is_not_an_error() {
        // The ordinary case. A workspace nobody ran anything in still closes.
        let mut tasks = TaskSet::new();
        assert_eq!(
            close_workspace(&WorkspaceId("empty".into()), 0, &mut tasks).len(),
            0
        );
    }

    #[test]
    fn a_term_registers_exactly_one_deadline_five_seconds_out() {
        // The grace period is read from the constant, never restated: a test with 5_000 written
        // into it passes after somebody changes the policy, which is the test agreeing with a
        // number rather than with a decision.
        let plan = stop_task(&TaskId("t".into()), TaskSignal::Term, 1_000).expect("planned");
        assert_eq!(plan.send, TaskSignal::Term);
        assert_eq!(plan.escalate_at, Some(1_000 + ESCALATION_GRACE_MS));
    }

    #[test]
    fn an_interrupt_does_not_escalate() {
        // SIGINT deliberately has no follow-up. A program that legitimately handles an interrupt
        // -- a REPL returning to its prompt, a build cancelling one target -- must not be killed
        // for having handled it.
        let plan = stop_task(&TaskId("t".into()), TaskSignal::Int, 1_000).expect("planned");
        assert_eq!(plan.send, TaskSignal::Int);
        assert_eq!(plan.escalate_at, None);
    }

    #[test]
    fn a_kill_has_nothing_to_escalate_to() {
        let plan = stop_task(&TaskId("t".into()), TaskSignal::Kill, 1_000).expect("planned");
        assert_eq!(plan.send, TaskSignal::Kill);
        assert_eq!(plan.escalate_at, None);
    }

    #[test]
    fn the_plan_carries_the_identity_it_was_made_for() {
        // The adapter keys the deadline on (TaskId, Pid). Losing the id here would leave it
        // keying on whatever it had to hand, which is how a deadline comes to kill a process
        // that merely inherited a reused name.
        let plan = stop_task(&TaskId("build-01".into()), TaskSignal::Term, 0).expect("planned");
        assert_eq!(plan.id, TaskId("build-01".into()));
    }

    #[test]
    fn a_deadline_is_relative_to_now_rather_than_to_zero() {
        // A clock read once at startup and never again produces deadlines in the past, and every
        // SIGTERM then escalates immediately -- which looks like a working escalation until
        // somebody notices nothing is ever given its grace period.
        let early = stop_task(&TaskId("t".into()), TaskSignal::Term, 0).expect("planned");
        let late = stop_task(&TaskId("t".into()), TaskSignal::Term, 900_000).expect("planned");
        assert!(late.escalate_at > early.escalate_at);
    }

    #[test]
    fn a_resize_of_a_task_without_a_terminal_changes_nothing() {
        // US2.3b. Not an error and not a refusal the client hears about: a task with pipes has
        // no window, and a panel showing it is still right to report its size.
        assert_eq!(
            resize_task(Some(Shape::Pipes), 120, 40, None),
            ResizeOutcome::NoTerminal
        );
    }

    #[test]
    fn a_zero_dimension_is_refused_rather_than_forwarded() {
        // Some programs read a zero dimension as "no terminal" and change what they print, so
        // forwarding one alters a task's behaviour rather than its layout. A client reporting an
        // element it has not laid out yet sends exactly this.
        let shape = Some(Shape::Pty { cols: 80, rows: 24 });
        assert_eq!(resize_task(shape, 0, 40, None), ResizeOutcome::Refused);
        assert_eq!(resize_task(shape, 120, 0, None), ResizeOutcome::Refused);
        assert_eq!(resize_task(shape, 0, 0, None), ResizeOutcome::Refused);
    }

    #[test]
    fn a_resize_for_an_unknown_task_is_not_a_terminal_question() {
        assert_eq!(resize_task(None, 120, 40, None), ResizeOutcome::NoSuchTask);
    }

    #[test]
    fn writing_to_an_unknown_task_is_distinguishable_from_writing_nothing() {
        // Neither reaches the client -- a notification has no response -- but a log that cannot
        // tell them apart cannot answer "did my keystroke go anywhere".
        assert_eq!(write_input(b"x", None), InputOutcome::NoSuchTask);
    }

    #[test]
    fn an_inherited_variable_survives_a_supplied_environment() {
        // §4.8 merges **over** rather than replacing. An implementation that replaces loses PATH,
        // and every task spawned by absolute path still runs, so nothing else notices.
        let merged = merge_over(
            inherited(&[("PATH", "/usr/bin")]),
            &overrides(&[("A", "1")]),
        );
        assert_eq!(value(&merged, "PATH"), Some("/usr/bin"));
        assert_eq!(value(&merged, "A"), Some("1"));
    }

    #[test]
    fn a_supplied_variable_wins_over_an_inherited_one_of_the_same_name() {
        // "Over" has a direction. A merge letting the inherited value win passes the case above.
        let merged = merge_over(
            inherited(&[("KEY", "inherited")]),
            &overrides(&[("KEY", "supplied")]),
        );
        assert_eq!(value(&merged, "KEY"), Some("supplied"));
        assert_eq!(
            merged.iter().filter(|(k, _)| k == "KEY").count(),
            1,
            "the variable must appear once, not twice with the winner last"
        );
    }

    #[test]
    fn no_overrides_leaves_the_inherited_environment_alone() {
        let merged = merge_over(
            inherited(&[("PATH", "/usr/bin"), ("HOME", "/root")]),
            &overrides(&[]),
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(value(&merged, "HOME"), Some("/root"));
    }
}
