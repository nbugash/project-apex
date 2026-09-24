//! Outbound port: running a process.
//!
//! A capability, not a pseudo-terminal. The adapter behind it is the only file permitted to
//! name `nix` (`engine/tests/pty_confinement.rs` fails the build otherwise), on exactly the
//! terms `file_watcher.rs` sets for `inotify` -- and everything that decides anything, which is
//! how output is chunked, when a producer is slowed, what the retention bound admits and when
//! an escalation is due, sits above this line and is tested against an in-memory double with no
//! process at all.

use std::sync::Arc;

use apex_protocol::wire::Pid;

use crate::application::ports::clock::Millis;
use crate::domain::path::ResolvedPath;
use crate::domain::task::{Shape, Stream, TaskSignal};

/// `Send + Sync`, because it owns nothing between calls: any thread handling a `runTask` may
/// spawn through it.
///
/// Note the contrast with `FileWatcher`, which is `Send` and deliberately not `Sync`. That port
/// owns one descriptor and one thread owns it; this one is a factory.
pub trait TaskRunner: Send + Sync {
    /// Start a process and hand back the two halves of it.
    ///
    /// Everything that can fail about *starting* fails here, which is what makes FR-004 and
    /// SC-015 satisfiable: a command that cannot be started never becomes a task that ran.
    fn spawn(&self, request: &SpawnRequest<'_>) -> Result<SpawnedTask, SpawnFailure>;
}

/// What a task needs to exist. Borrowed throughout: nothing here outlives the call.
pub struct SpawnRequest<'a> {
    /// Program, then arguments. **Never a shell line** -- the engine interposes no `sh -c`. A
    /// caller wanting a shell names one as `command[0]`.
    pub command: &'a [String],
    /// Proven contained by construction (§4.7, FR-003). The port cannot be handed an unchecked
    /// path because `ResolvedPath` has no other constructor.
    pub cwd: &'a ResolvedPath,
    /// The engine's environment with the caller's merged over it, already combined by the use
    /// case. The port receives the final set and makes no policy from it.
    pub env: &'a [(String, String)],
    /// A terminal or separate pipes. Exclusive by construction -- see `Shape`.
    pub shape: Shape,
    /// Per-process ceilings, applied between fork and exec (FR-006, A-TASKLIMIT).
    pub limits: ResourceLimits,
}

/// A started process, split so that reading cannot block writing.
///
/// **T12.** The split is the whole reason a keystroke does not wait behind a blocked read. One
/// trait with `&mut self` would need a mutex around the runner, and a `writeStdin` would then
/// queue behind a `read` that is blocked precisely because the process has nothing to say --
/// which is FR-012 failing through the shape of the port rather than through anything anyone
/// wrote.
pub struct SpawnedTask {
    /// The process group leader's id. §4.8's `runTask` result.
    pub pid: Pid,
    /// Owned by this task's reader thread and never shared.
    pub output: Box<dyn TaskOutput>,
    /// Shared with whatever dispatches `writeStdin`, `resizePty` and `terminate`.
    ///
    /// **T13.** Reachable without the task map's lock, so a `workspace/close` reaches a task
    /// whose reader is blocked in `read` rather than queueing behind it.
    pub control: Arc<dyn TaskControl>,
}

/// The reading half. `Send`, **not** `Sync`: one thread owns it, and two threads draining one
/// descriptor lose bytes between them -- the silence FR-009 and FR-010 forbid.
pub trait TaskOutput: Send {
    /// Block for at most `timeout`, then append whatever arrived to `out`.
    ///
    /// Appending into a caller-owned buffer rather than returning a fresh `Vec` means an idle
    /// task allocates nothing per read. The timeout is the time remaining until the chunker's
    /// next due emission, so the thread wakes to flush and for nothing else.
    fn read(&mut self, timeout: Millis, out: &mut Vec<u8>) -> ReadOutcome;
}

/// The controlling half. `Send + Sync` behind an `Arc`.
pub trait TaskControl: Send + Sync {
    fn write_stdin(&self, data: &[u8]) -> Result<(), ControlError>;

    /// Silently ignored for a task started with `Shape::Pipes`: there is no terminal to resize,
    /// and `resizePty` is a notification, which has no way to refuse.
    fn resize(&self, cols: u16, rows: u16) -> Result<(), ControlError>;

    /// To the process **group**, never the bare pid. A shell's children go with it, which is
    /// how FR-018 is met without a cgroup.
    fn signal(&self, signal: TaskSignal) -> Result<(), ControlError>;

    /// `None` while the process is still running. Non-blocking.
    fn reap(&self) -> Option<Exit>;
}

/// What one read produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// `len` bytes were appended to `out`, from `stream`.
    Bytes { stream: Stream, len: usize },
    /// The timeout expired with nothing to report. Not an error, and nothing branches on it
    /// beyond flushing a chunk that has come due.
    Idle,
    /// End of output. **Every byte the process wrote has already been returned**, which is what
    /// FR-022 and SC-011 rest on: the exit cannot be reported before the output it followed.
    Ended,
    /// The descriptor itself failed. `ErrorKind` rather than `io::Error` so the enum stays
    /// `Copy` and `PartialEq` and a test can assert on the failure it provoked.
    Failed(std::io::ErrorKind),
}

/// How a task ended. **Two states, not one field with a convention** (FR-021, SC-010). A
/// shell's `128 + n` is exactly what this type exists to make unwritable.
///
/// **`Signal` carries the raw number, not a `TaskSignal`, and the asymmetry is the point.**
/// `TaskSignal` is what a client may ask this feature to *send*, and it is closed at three.
/// What *kills* a task is the host's whole vocabulary: `SIGSEGV` from a compiler bug, `SIGPIPE`
/// from a closed pager, `SIGKILL` from the out-of-memory killer. FR-020 requires "the signal
/// that killed it", so a segfaulting build -- an ordinary case, not an exotic one -- must be
/// reportable. A three-variant `Signal` has nothing to put in it, and the two shapes that
/// remain are both forbidden: dropping the death, or encoding it as `Code(139)`, which is the
/// convention this type exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// Why a process could not be started.
///
/// Every variant becomes `-32011` on the wire, because FR-004 and SC-015 admit one outcome for
/// a command that could not be started and §4.4 gives that outcome one code. The variant
/// survives as the reason a user is shown, and the distinction that matters to them is whose
/// fault it is: `NotExecutable` and `CwdUnusable` are the developer's to fix, `NoDevice` and
/// `LimitRefused` are the instance's condition. **No reason may carry the environment**
/// (FR-005a, SC-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnFailure {
    /// `command[0]` was not found on `PATH`, or is not executable.
    NotExecutable,
    /// The working directory was resolved and is gone, or is not a directory, by the time the
    /// child tried to enter it. A race, not a bug.
    CwdUnusable,
    /// A terminal or a pipe could not be allocated -- descriptors exhausted, or no pty free.
    NoDevice,
    /// A limit in `ResourceLimits` was refused by the kernel. Distinct from the process later
    /// exceeding one, which is an ordinary death.
    LimitRefused,
    Failed(std::io::ErrorKind),
}

/// Why a control operation did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlError {
    /// The process has ended. **Not an error the caller surfaces**: a notification has no
    /// response, and FR-019 makes terminating an exited task a success.
    Gone,
    Failed(std::io::ErrorKind),
}

/// The per-process ceilings a task runs under.
///
/// Exactly the limits plan.md's *Fixed Quantities* states, and no field for a limit it
/// deliberately declined -- there is no CPU time here, and no process count, because both were
/// refused with reasons rather than forgotten. `FIXED` is what every task gets; the struct
/// exists so a test can construct another and provoke `LimitRefused`, not so a caller can tune
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Address space a single process may map, set as **both the soft and the hard limit**.
    ///
    /// Both, because a child may raise its own soft limit up to its hard limit: a soft-only
    /// ceiling is one the bounded process can simply remove, and the removal is invisible until
    /// the first measurement of a runaway finds it was never bounded at all.
    pub address_space_bytes: u64,
    /// Core dump size. **Zero, and required to be.** FR-005a forbids a task's environment
    /// reaching any log or crash report, and a core dump is a crash report carrying the whole
    /// environment. A requirement satisfied by a limit, not a limit chosen for tuning, and the
    /// one field here that may not be varied by a caller for any reason.
    pub core_bytes: u64,
}

impl ResourceLimits {
    /// plan.md's *Fixed Quantities*: 16 GiB of address space, soft and hard; no core dumps.
    pub const FIXED: Self = Self {
        address_space_bytes: 16 * 1024 * 1024 * 1024,
        core_bytes: 0,
    };
}
