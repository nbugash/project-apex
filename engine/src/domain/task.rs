//! What a task is, and what the engine knows about one.
//!
//! Nothing here spawns a process, reads a descriptor or asks the time. A task's identity, its
//! shape, how it ended and whether it is still live are all decidable from values, which is what
//! lets the requirements about them -- a second start under a live identity is refused, a
//! release frees the identity, a drain is not repeatable -- be arithmetic rather than a test
//! that spawns something.
//!
//! `RetainedOutput` is deliberately **not** here, and `Task` does not hold one. It carries a
//! bound taken from plan.md, and F004 put exactly that kind of thing in the application layer:
//! `WatchSet` lives in this module's neighbour and the window and threshold it is governed by
//! live in `application/coalescer.rs`. The task service pairs a `TaskSet` with per-task
//! retention the same way the watch service pairs a `WatchSet` with a `Coalescer`.

use std::collections::BTreeMap;

use apex_protocol::wire::{Pid, TaskId};

/// Which of a task's two output descriptors a chunk came from.
///
/// Still two variants when a task has a pseudo-terminal, where everything arrives on `Stdout`.
/// That is A-TASKSTREAM as a shape: the merge happens because a terminal is one device, not
/// because the engine stopped distinguishing, and a replay has to put each chunk back on the
/// notification it would have used when live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// What a task's output is attached to.
///
/// An enum and not the wire's `pty` boolean plus two optional dimensions. FR-008a makes the
/// choice exclusive, and a bool beside `cols`/`rows` is a shape in which both can be set at
/// once -- a state the type system would let through and every reader would then have to
/// re-decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// §4.8 defaults an omitted size to 80 x 24. Never 0 x 0, which is the kernel's default for
    /// a fresh pseudo-terminal, a size no display has, and the one value `resizePty` refuses.
    Pty {
        cols: u16,
        rows: u16,
    },
    Pipes,
}

/// What a client may ask `execution/terminate` to send: the **sending** vocabulary, closed at
/// three.
///
/// Deliberately not the vocabulary a task can *die* of, which is open -- `SIGSEGV` from its own
/// bug, `SIGKILL` from the out-of-memory killer. `ExitStatus::Signalled` carries a number for
/// that reason and this does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSignal {
    Int,
    Term,
    Kill,
}

/// How a task ended: two distinct states, never one field carrying `128 + n`.
///
/// The shell convention encodes a signal into an exit code for a human reading a number. A
/// protocol that did the same would make 143 ambiguous between a stop the developer asked for
/// and a program that chose to exit 143, and FR-021 requires the two to be distinguishable in
/// every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Exited {
        code: i32,
    },
    /// The number the kernel delivered, whatever it was. Named for the wire by
    /// `SignalName::from_number`, which is total.
    Signalled {
        signal: i32,
    },
}

/// One read from a task's output, tagged with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputChunk {
    pub stream: Stream,
    pub bytes: Vec<u8>,
}

impl OutputChunk {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Whether a task is still running, and how it ended if not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Ended(ExitStatus),
}

/// The variables a caller asked to set, merged **over** the engine's environment rather than
/// replacing it.
///
/// A newtype whose own `Debug` elides its contents, and not a hand-written `Debug` on `Task`.
/// FR-005a keeps a task's environment out of every log and crash report because it commonly
/// carries credentials, and a `Debug` written on the containing struct is defeated the next
/// time somebody adds a field and reaches for `#[derive(Debug)]`. Putting the elision on the
/// type means the secret cannot be printed by anything, anywhere, however it is reached.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EnvOverrides(BTreeMap<String, String>);

impl EnvOverrides {
    pub fn new(vars: BTreeMap<String, String>) -> Self {
        Self(vars)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for EnvOverrides {
    /// The count and nothing else. A count is useful when reading a log and cannot leak a token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EnvOverrides({} vars, elided)", self.0.len())
    }
}

/// One task the engine is holding.
///
/// No window-size field. The current size lives in the kernel's `winsize` for the
/// pseudo-terminal, and a copy here would be a second authority that can disagree with the
/// first -- a resize that reached the kernel and not this struct, or the reverse, and no way to
/// say which is right. `Shape::Pty` records the size the task was *created* with, which is a
/// different fact and one nothing else holds.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub workspace: apex_protocol::wire::WorkspaceId,
    pub command: Vec<String>,
    pub shape: Shape,
    pub pid: Pid,
    pub env: EnvOverrides,
    pub state: TaskState,
}

impl Task {
    pub fn is_running(&self) -> bool {
        matches!(self.state, TaskState::Running)
    }
}

/// Every task the engine is holding. One per engine, not one per workspace.
///
/// A task identity is engine-unique (§4.8), so a per-workspace set could not answer the six
/// methods that address a bare `task_id`. Never given a reference to the transport: what is
/// written and when is the adapter's business, and a domain type that could write would be a
/// domain type that could deadlock.
#[derive(Debug, Default)]
pub struct TaskSet {
    tasks: BTreeMap<TaskId, Task>,
}

/// Why a start was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefused {
    /// The identity is live. The caller's answer is to attach, not to retry.
    AlreadyRunning,
}

impl TaskSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deliberately **not** idempotent, where F004's `WatchSet::acquire` is.
    ///
    /// Watching a path twice is the same request twice and answering it once is right. Starting
    /// a task twice is two processes, and FR-031c requires the second to be refused
    /// *distinguishably* -- a client racing its own reconnection must not be told it attached
    /// when it in fact started a second build. The refusal happens here, before anything is
    /// spawned, so the process that must not exist is never created.
    pub fn start(&mut self, task: Task) -> Result<(), StartRefused> {
        if self.tasks.contains_key(&task.id) {
            return Err(StartRefused::AlreadyRunning);
        }
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn get_mut(&mut self, id: &TaskId) -> Option<&mut Task> {
        self.tasks.get_mut(id)
    }

    pub fn contains(&self, id: &TaskId) -> bool {
        self.tasks.contains_key(id)
    }

    /// Frees the identity. After this a client may start a new task under the same name, which
    /// is ordinary -- re-running `build` in the same panel is what a developer does all day.
    pub fn release(&mut self, id: &TaskId) -> Option<Task> {
        self.tasks.remove(id)
    }

    /// Ordered by `TaskId`, for determinism only. Nothing depends on the order and a test
    /// asserting on a listing should not depend on hash iteration.
    pub fn list(&self, workspace: Option<&apex_protocol::wire::WorkspaceId>) -> Vec<&Task> {
        self.tasks
            .values()
            // `map_or(true, ..)` and not `is_none_or`, which is stable only since 1.82 against a
            // declared MSRV of 1.75.
            .filter(|t| workspace.map_or(true, |ws| &t.workspace == ws))
            .collect()
    }

    /// Removes and returns every task of one workspace. Draining twice yields nothing the
    /// second time, which is what makes a second `workspace/close` answerable rather than a
    /// silent repeat.
    pub fn drain_for_workspace(
        &mut self,
        workspace: &apex_protocol::wire::WorkspaceId,
    ) -> Vec<Task> {
        let ids: Vec<TaskId> = self
            .tasks
            .values()
            .filter(|t| &t.workspace == workspace)
            .map(|t| t.id.clone())
            .collect();
        ids.iter().filter_map(|id| self.tasks.remove(id)).collect()
    }

    /// Removes and returns everything. The engine's own re-execution calls this (A-TASKEXEC).
    pub fn drain_all(&mut self) -> Vec<Task> {
        std::mem::take(&mut self.tasks).into_values().collect()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_protocol::wire::WorkspaceId;

    fn task(id: &str, ws: &str) -> Task {
        Task {
            id: TaskId(id.into()),
            workspace: WorkspaceId(ws.into()),
            command: vec!["cargo".into(), "test".into()],
            shape: Shape::Pty { cols: 80, rows: 24 },
            pid: Pid(4242),
            env: EnvOverrides::default(),
            state: TaskState::Running,
        }
    }

    #[test]
    fn a_second_start_under_a_live_identity_is_refused() {
        let mut set = TaskSet::new();
        assert!(set.start(task("build", "ws")).is_ok());
        assert_eq!(
            set.start(task("build", "ws")),
            Err(StartRefused::AlreadyRunning)
        );
        // The point of refusing here rather than after spawning: there is exactly one task, so
        // there was exactly one process. FR-031c is about the process that must not exist.
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn releasing_frees_the_identity_for_reuse() {
        let mut set = TaskSet::new();
        set.start(task("build", "ws")).expect("start");
        assert!(set.release(&TaskId("build".into())).is_some());
        // Re-running `build` in the same panel is what a developer does all day.
        assert!(set.start(task("build", "ws")).is_ok());
    }

    #[test]
    fn releasing_something_never_started_is_not_an_error() {
        let mut set = TaskSet::new();
        assert!(set.release(&TaskId("never".into())).is_none());
    }

    #[test]
    fn draining_a_workspace_takes_only_that_workspace() {
        let mut set = TaskSet::new();
        set.start(task("a", "ws-1")).expect("start");
        set.start(task("b", "ws-1")).expect("start");
        set.start(task("c", "ws-2")).expect("start");

        let drained = set.drain_for_workspace(&WorkspaceId("ws-1".into()));
        assert_eq!(drained.len(), 2);
        assert_eq!(set.len(), 1, "ws-2's task must survive ws-1 closing");

        // Draining twice yields nothing the second time. A second `workspace/close` is
        // answerable because of this, rather than being a silent repeat of the first.
        assert!(set
            .drain_for_workspace(&WorkspaceId("ws-1".into()))
            .is_empty());
    }

    #[test]
    fn draining_everything_twice_yields_nothing_the_second_time() {
        let mut set = TaskSet::new();
        set.start(task("a", "ws-1")).expect("start");
        set.start(task("b", "ws-2")).expect("start");
        assert_eq!(set.drain_all().len(), 2);
        assert!(set.drain_all().is_empty());
        assert!(set.is_empty());
    }

    #[test]
    fn listing_filters_by_workspace_and_is_ordered() {
        let mut set = TaskSet::new();
        set.start(task("zebra", "ws-1")).expect("start");
        set.start(task("alpha", "ws-1")).expect("start");
        set.start(task("other", "ws-2")).expect("start");

        let all = set.list(None);
        assert_eq!(all.len(), 3);
        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "other", "zebra"], "ordered by id");

        let scoped = set.list(Some(&WorkspaceId("ws-1".into())));
        assert_eq!(scoped.len(), 2);
    }

    /// The assertion SC-025 rests on, and the one `#[derive(Debug)]` silently breaks.
    #[test]
    fn debug_shows_neither_the_key_nor_the_value_of_an_environment_variable() {
        let mut vars = BTreeMap::new();
        vars.insert("AWS_SECRET_ACCESS_KEY".to_string(), "hunter2".to_string());
        let mut t = task("build", "ws");
        t.env = EnvOverrides::new(vars);

        let rendered = format!("{t:?}");
        assert!(
            !rendered.contains("hunter2"),
            "the value reached a log: {rendered}"
        );
        assert!(
            !rendered.contains("AWS_SECRET_ACCESS_KEY"),
            "the key reached a log, which names the secret even without its value: {rendered}"
        );
        // The count survives, because it is useful when reading a log and cannot leak anything.
        assert!(rendered.contains("1 vars, elided"), "{rendered}");
    }

    #[test]
    fn an_exit_by_signal_is_a_different_state_from_an_exit_by_code() {
        let signalled = ExitStatus::Signalled { signal: 15 };
        let exited = ExitStatus::Exited { code: 143 };
        // 143 is 128 + 15. Under the shell convention these would be the same value; here they
        // are different states, which is what FR-021 asks for.
        assert_ne!(signalled, exited);
    }
}
