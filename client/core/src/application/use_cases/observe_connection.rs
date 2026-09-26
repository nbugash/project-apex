//! Bridge connection-source transitions to interface events.

use crate::application::ports::connection::{ConnectionStatusSource, StateSink};
use crate::application::ports::task_provider::TaskProvider;
use crate::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use crate::application::use_cases::observe_task::Ending;
use crate::domain::connection::ConnectionState;
use crate::domain::workspace::{RelPath, WorkspaceId};
use apex_protocol::wire::TaskId;
use std::sync::Arc;

pub struct ObserveConnection {
    source: Arc<dyn ConnectionStatusSource>,
}

impl ObserveConnection {
    pub fn new(source: Arc<dyn ConnectionStatusSource>) -> Self {
        Self { source }
    }

    pub fn current(&self) -> ConnectionState {
        self.source.current()
    }

    pub fn start(&self, sink: StateSink) {
        self.source.subscribe(sink);
    }
}

/// What the reattachment did, step by step.
///
/// Recorded rather than inferred, because the **order** is the claim. Every step here is
/// observable from the outside as a round trip, and a sequence that looks right in the code can
/// still be wrong in the calls it makes -- which is the only place it matters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Re-watching the workspace's paths. The engine forgot them when the connection dropped;
    /// the client did not, so this is the client telling it again rather than asking what it had.
    Watch,
    /// Enumerating tasks. **Only when the identities were lost.**
    List,
    Attach(TaskId),
    /// Telling a task its size. Attaching deliberately sets none (attach guarantee 9).
    Resize(TaskId),
}

/// How a remembered task turned out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still running, and reattached. `retained` is how much was missed.
    Survived { task: TaskId, retained: u64 },
    /// Finished while the client was away, and this is how.
    Finished { task: TaskId, ending: Ending },
    /// The engine no longer holds this identity.
    ///
    /// **Surfaced, not retried.** `-32006` means the identity is released, so attaching again
    /// asks the same question and gets the same answer -- a retry loop here is a client telling
    /// the developer nothing while it waits for a task that is gone.
    Gone { task: TaskId },
}

/// What the developer is told after a reconnection (FR-032).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reattachment {
    pub steps: Vec<Step>,
    pub outcomes: Vec<Outcome>,
}

impl Reattachment {
    /// Bytes missed across every task that survived, which is the number worth reporting.
    pub fn retained(&self) -> u64 {
        self.outcomes
            .iter()
            .map(|o| match o {
                Outcome::Survived { retained, .. } => *retained,
                _ => 0,
            })
            .sum()
    }
}

/// Reattach to the tasks this client left running.
///
/// The workspace is **already registered** when this runs: reattaching to a workspace the engine
/// has never heard of is not a thing that can succeed, so registration is a precondition rather
/// than a step -- the type says so by taking an id that only exists once registration returned it.
///
/// `execution/list` is called **only when the identities were lost**. A client that has them and
/// lists first has spent a round trip learning what it already knew, on the one code path where
/// the developer is waiting and the link has just proved unreliable.
///
/// Every attached task is then told its size, because attaching deliberately sets none -- a
/// client that forgets leaves the task laying out to the width of a window that has since been
/// resized or closed.
pub async fn reattach(
    workspace: &WorkspaceId,
    watch_paths: &[RelPath],
    remembered: &[TaskId],
    size: (u16, u16),
    workspaces: &dyn WorkspaceProvider,
    tasks: &dyn TaskProvider,
) -> Reattachment {
    let mut report = Reattachment::default();

    // Re-watch first. The engine forgot the watch set with the connection, and a client that
    // attached before re-watching would receive a build's output while believing the files it
    // touches are unchanged.
    let _ = workspaces.watch(workspace, watch_paths).await;
    report.steps.push(Step::Watch);

    let identities: Vec<TaskId> = if remembered.is_empty() {
        report.steps.push(Step::List);
        tasks
            .list(Some(workspace))
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.task_id)
            .collect()
    } else {
        remembered.to_vec()
    };

    for task in identities {
        report.steps.push(Step::Attach(task.clone()));
        match tasks.attach(workspace, &task).await {
            Ok(result) if result.running => {
                report.outcomes.push(Outcome::Survived {
                    task: task.clone(),
                    retained: result.retained,
                });
                // Only a running task has a terminal to resize. Sending one to a task that has
                // finished is a notification the engine drops, and a round trip spent on it.
                let _ = tasks.resize(&task, size.0, size.1).await;
                report.steps.push(Step::Resize(task));
            }
            Ok(result) => {
                let ending = match (result.exit_code, result.signal) {
                    (Some(code), None) => Ending::Code(code),
                    (None, Some(name)) => Ending::Signal(name),
                    _ => Ending::Unintelligible,
                };
                report.outcomes.push(Outcome::Finished { task, ending });
            }
            // Released, so asking again asks the same question. Told to the developer instead.
            Err(ProviderError::TaskNotFound) => {
                report.outcomes.push(Outcome::Gone { task });
            }
            Err(_) => {
                // A transport fault is not an answer about the task. Reported as gone would be a
                // lie; left out, the summary says nothing about it, which is the honest shape
                // until the connection is good enough to ask again.
            }
        }
    }
    report
}
