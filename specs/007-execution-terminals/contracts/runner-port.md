# Contract: `TaskRunner`

**Feature**: F010 execution-terminals | **Date**: 2026-09-24

One outbound port the engine acquires. It is a **capability, not a technology** (Principle VIII):
`TaskRunner`, not `PtyRunner`. It reuses the `Clock` port F004 added
(`engine/src/application/ports/clock.rs`), **widened** here to `Send + Sync` with a `sleep_until`
method, and adds no second notion of time.

This document states signatures and the guarantees a signature cannot carry. **There are no bodies
here and none are implied** — the adapter is `engine/src/adapters/outbound/pty_runner.rs` and
nothing else, guarded by `engine/tests/pty_confinement.rs` the way `inotify_confinement.rs` guards
F004's watcher.

Rationale for the shapes below is in research.md, *The pseudo-terminal mechanism*, *A thread per
task*, *Backpressure comes for free* and *Confining the mechanism*. It is not restated.

**Synchronous, and that is not an accident.** `engine/Cargo.toml` records the constraint in its
own comment: the engine "stays synchronous and runtime-free, because it is embedded in the client
and transferred on every first connect", and repeats it beside F004's dependency — "Still no async
runtime: the watcher is one std thread polling a descriptor." A pseudo-terminal is not a reason to
reverse a decision made about binary size and transfer cost. There is one descriptor per task and
one thread per task; `std::thread` plus a read with a timeout does the whole job.
**No `async`, no `tokio`, anywhere in this contract.** The precedents are
`application/ports/file_system.rs` and `application/ports/file_watcher.rs`, both synchronous for
the same reason and both saying so.

Rust MSRV is **1.75**, declared identically by `protocol`, `engine` and `client/core`. Nothing
below uses a language or stdlib feature newer than that.

---

## The dividing line

plan.md states the structural rule and it is the reason the port exists: **the pseudo-terminal is
named in exactly one file, and everything that decides anything is testable without a process.**
That is what makes FR-010, FR-011, FR-013 and SC-005 unit tests rather than integration tests with
a build in them.

| Behind the port — `pty_runner.rs` | In pure application code — `application/output.rs`, `use_cases/task.rs` |
|---|---|
| Opening the pseudo-terminal pair, or the three pipes | Chunking by the size bound and the time bound (FR-011, SC-005) |
| `fork`/`exec`, the process group, the controlling terminal | Deciding when a chunk is due, from the `Clock` port |
| Applying resource limits between fork and exec (FR-006) | Ordering within a task (FR-010, SC-004) |
| Blocking on the descriptor with a timeout | Holding retained output and enforcing its bound (FR-013a, FR-031a) |
| Mapping `EIO` on a pty master to `ReadOutcome::Ended` | Deciding to stop reading, which is the whole of backpressure (FR-013) |
| Window-size changes and the `SIGWINCH` that follows | Which attachment is owed what, and what has been delivered (FR-031b) |
| Signalling the process **group** (FR-006a, FR-018) | Refusing a live identity (FR-031c), releasing a dead one (FR-023) |
| `waitpid`, and caching the status once collected | Building the wire notification from a chunk (task-events.md) |
| — | Enumerating live tasks and which workspace owns each (`execution/list`, SC-023) |
| — | Selecting a workspace's tasks and **choosing** the stop escalation's deadline (`workspace/close`, FR-024). The five-second policy is application code; the **waiting** is the escalation thread's, an adapter, because a use case that sleeps blocks the dispatch thread (design.md CONFLICT 8) |
| Nothing else | Everything else |

Three entries are worth naming because they look like adapter work and are not.

**Backpressure is not behind the port.** The port offers a read; the application decides not to
call it. That is the entire mechanism (research.md, *Backpressure comes for free*), and putting it
behind the port would make SC-021 need a process that can outrun a link.

**The retention bound is not behind the port.** The port has no memory of what it has already
handed over. FR-013a's bound, FR-031a's identical treatment of a detached task, and SC-019's
ordering across a reattachment are all decisions about bytes the application already holds.

**Path containment is not behind the port.** `SpawnRequest` carries a `&ResolvedPath`, whose only
public constructor is `resolve` (`engine/src/domain/path.rs`), so the port cannot be handed a
working directory whose containment was never checked (§4.7, FR-003, Principle VI). This is F004's
`FileWatcher::watch` taking a `ResolvedPath`, applied to a second port for the same reason.

---

## `TaskRunner`

`engine/src/application/ports/task_runner.rs`. The types it yields live in
`engine/src/domain/task.rs`, per plan.md's source layout.

```rust
use crate::application::ports::clock::Millis;
use crate::domain::path::ResolvedPath;
use crate::domain::task::{Exit, Pid, Shape, Stream, TaskSignal};
use std::sync::Arc;

/// Spawning a process, as a capability.
///
/// `Send + Sync` because the composition root shares one of these behind an `Arc` and any
/// thread that handles a `runTask` may spawn through it. Note the contrast with `FileWatcher`,
/// which is `Send` and deliberately not `Sync`: that port owns one descriptor and one thread
/// owns it, whereas this one owns nothing between calls.
pub trait TaskRunner: Send + Sync {
    /// Start a process and hand back the two halves of it.
    ///
    /// Everything that can fail about starting fails here, which is what makes FR-004 and
    /// SC-015 satisfiable: a command that cannot be started never becomes a task that ran.
    fn spawn(&self, request: &SpawnRequest<'_>) -> Result<SpawnedTask, SpawnFailure>;
}

/// What a task needs to exist. Borrowed throughout: nothing here outlives the call.
pub struct SpawnRequest<'a> {
    /// Program, then arguments. **Never a shell line** — the engine interposes no `sh -c`
    /// (task-methods.md, `runTask` guarantee 1). A client wanting a shell names one.
    pub command: &'a [String],
    /// Proven contained by construction (§4.7, FR-003).
    pub cwd: &'a ResolvedPath,
    /// The engine's environment with the caller's applied over it, already merged by the use
    /// case (spec Assumptions). The port receives the final set and makes no policy from it.
    pub env: &'a [(String, String)],
    /// A terminal or separate pipes. Exclusive by construction — see `Shape`.
    pub shape: Shape,
    /// Per-process ceilings, applied between fork and exec (FR-006, A-TASKLIMIT).
    pub limits: ResourceLimits,
}

/// A started process, split so that reading cannot block writing.
pub struct SpawnedTask {
    /// The process group leader's id. §4.8's `runTask` result, and what §15.2 tracks.
    pub pid: Pid,
    /// Owned by this task's reader thread and never shared (research.md, *A thread per task*).
    pub output: Box<dyn TaskOutput>,
    /// Shared with whatever dispatches `writeStdin`, `resizePty` and `terminate`.
    pub control: Arc<dyn TaskControl>,
}

/// The reading half. `Send`, **not** `Sync`: one thread owns it, and two threads draining one
/// descriptor lose bytes between them — the silence FR-009 and FR-010 forbid.
pub trait TaskOutput: Send {
    /// Block for at most `timeout`, then append whatever arrived to `out`.
    ///
    /// Returning into a caller-owned buffer rather than a fresh `Vec` means an idle task
    /// allocates nothing per read. The timeout is the time remaining until the chunker's next
    /// due emission, so the thread wakes to flush and for nothing else.
    fn read(&mut self, timeout: Millis, out: &mut Vec<u8>) -> ReadOutcome;
}

/// The controlling half. `Send + Sync` because a keystroke, a resize and a stop all arrive on
/// the dispatch thread while the reader thread is blocked in `read` — and none of them may
/// wait for a read to return (FR-012, SC-006).
pub trait TaskControl: Send + Sync {
    /// Write bytes to the process's input, unmodified (FR-014, SC-007).
    ///
    /// Returns when every byte has been handed to the kernel or the write failed. A partial
    /// write is retried behind this call rather than surfaced, because a caller that has to
    /// resume a partial write is a caller that can reorder one client's keystrokes.
    fn write_stdin(&self, data: &[u8]) -> Result<(), ControlError>;

    /// Tell the process its terminal is a different size (FR-016, SC-009).
    ///
    /// `Ok(())` and no effect for `Shape::Pipes`: there is no terminal to resize, and a caller
    /// switching shapes must not have to special-case it (task-methods.md, `resizePty`
    /// guarantee 3).
    fn resize(&self, cols: u16, rows: u16) -> Result<(), ControlError>;

    /// Send a signal to the process **group** (FR-006a, FR-018, SC-027).
    ///
    /// Not to the pid. Signalling the leader alone leaves the compilers it spawned running,
    /// which is the spec's *A process that spawns children* edge case.
    fn signal(&self, signal: TaskSignal) -> Result<(), ControlError>;

    /// Collect the exit status if the process has ended, without blocking.
    ///
    /// **Idempotent, and that is load-bearing.** The reader thread calls it after `Ended` and a
    /// `terminate` may call it after a kill; a real `waitpid` succeeds once, so the adapter
    /// caches the status and every later call returns the same `Some`. Without that, whichever
    /// caller loses the race sees a task that never ended.
    fn reap(&self) -> Option<Exit>;
}
```

### What the port yields

```rust
/// A host process id. A newtype so nothing outside the adapter does arithmetic on it, and so
/// the negation that turns a pid into a process group cannot be written by accident above the
/// line that is allowed to know the two are related.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pid(pub i32);

/// Which shape the caller chose, and therefore which stream can exist.
///
/// **An enum rather than the wire's `pty` boolean, because FR-008a requires the choice to be
/// exclusive and a bool plus two optional dimensions is a shape in which both can be set.**
/// The wire carries the boolean §4.8 defines; the domain carries the thing it means
/// (A-TASKSTREAM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One device. `isatty` is true and both descriptors write into it, so only
    /// `Stream::Stdout` can ever be yielded (guarantee T3).
    ///
    /// The dimensions are needed at creation: a terminal has a size before anything resizes
    /// it. §4.8's `runTask` now carries `cols?` and `rows?` for exactly this, so the use case
    /// passes what the client chose — and **80 × 24 when the client chose nothing**, which §4.8
    /// states and plan.md's *Fixed Quantities* fixes (task-methods.md, `runTask` guarantee 7,
    /// and *What was open here, and is not any more*, item 1). Both values reach this port
    /// already decided: the fallback is applied in the use case, so the port still invents no
    /// size and the quantity stays where FR-006b requires it. What the port must never be
    /// passed is 0 × 0, the kernel's own default for a new pseudo-terminal — a size no display
    /// has, and the one value `execution/resizePty` deliberately refuses to set.
    Pty { cols: u16, rows: u16 },
    /// Three pipes. `isatty` is false and the two output streams stay distinguishable.
    Pipes,
}

/// Which descriptor a read came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// What one read produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// `len` bytes were appended to `out`, from `stream`.
    Bytes { stream: Stream, len: usize },
    /// The timeout expired with nothing to report. Not an error, and not a condition any
    /// caller branches on beyond flushing a due chunk.
    Idle,
    /// End of output. **Every byte the process wrote has already been returned** — see
    /// guarantee T4, which is what FR-022 and SC-011 rest on.
    Ended,
    /// The descriptor itself failed. `ErrorKind` rather than `io::Error` so the enum stays
    /// `Copy` and `PartialEq` and a test can assert on the failure it provoked.
    Failed(std::io::ErrorKind),
}

/// How a task ended. **Two states, not one field with a convention** (spec, Key Entities;
/// FR-021, SC-010). A shell's `128 + n` is exactly what this type exists to make unwritable.
///
/// **`Signal` carries the raw number, not a `TaskSignal`, and the asymmetry is the point.**
/// `TaskSignal` is what a client may ask this feature to *send*, and it is closed at three.
/// What *kills* a task is the host's whole vocabulary: `SIGSEGV` from a compiler bug, `SIGPIPE`
/// from a closed pager, `SIGHUP`, `SIGKILL` from the out-of-memory killer. FR-020 requires "the
/// signal that killed it" and SC-010 requires a signal death distinguishable in 100% of
/// exercised cases, so a segfaulting build — an ordinary case, not an exotic one — must be
/// reportable. A three-variant `Signal` has no value to put in it, and the shapes that remain
/// are both forbidden: dropping the death, or encoding it as `Code(139)`, which is the `128 + n`
/// convention this type exists to make unwritable. The adapter names the number for the wire
/// (§4.8: the name, never the number); `data-model.md`'s `ExitStatus::Signalled { signal: i32 }`
/// is the same value one layer up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// The signals this feature sends. A closed enum, so the port cannot be handed an arbitrary
/// integer from a wire frame — the same reasoning `ResolvedPath` applies to paths.
///
/// **It closes the *sending* vocabulary only.** This type is the parameter of
/// `TaskControl::signal` and of nothing else: it is where a caller *chooses* a signal, which is
/// the one place an open set would let a wire frame pick a syscall's argument. What a task was
/// *killed* by is `Exit::Signal(i32)`, open by necessity — see `Exit`. Two vocabularies, two
/// directions, and conflating them costs FR-020 the signals nobody asked for.
///
/// **The vocabulary is closed at three and the escalation is fixed** (§4.8; plan.md, *Fixed
/// Quantities*). `Int` is FR-015's interrupt — `SIGINT`, what Ctrl-C sends; `Term` is FR-017's
/// stop; `Kill` is FR-018's guarantee that it ends. The wire carries the **name** and not the
/// number, because numbers differ between platforms and the client is not always on the engine's;
/// this enum is where the name stops and the number begins, and the number exists only inside the
/// adapter. On the wire it is `protocol`'s `TerminateSignal`, whose variants carry per-variant
/// `#[serde(rename = "SIGINT")]` and not `rename_all = "UPPERCASE"`, which would spell `"INT"`.
///
/// A `Term` escalates to a `Kill` after **5 s**, sent to the process group; an `Int` does not
/// escalate, because a program that legitimately handles it must not be killed for having handled
/// it. The same `Term`-then-`Kill` rule ends a workspace's tasks, where no caller named anything
/// (`workspace/close`, FR-024) and the engine begins at `Term`.
///
/// **The escalation is not behind this port.** The use case sends `Term` and **registers a
/// deadline**; a single escalation thread waits on `Clock::sleep_until` and sends the `Kill`.
/// Both halves are decisions and therefore live above the line (*The dividing line*). An
/// earlier version of this comment called it "two `signal` calls with a `Clock` wait between
/// them", which describes a dispatch thread blocking for five seconds per stop — the shape
/// design.md **[CONFLICT 8]** replaced. A port that escalated on its own would make those five
/// seconds untestable without a real process that ignores `SIGTERM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSignal {
    Int,
    Term,
    Kill,
}

/// Why a process could not be started. This is FR-004's failure, and it never becomes a task.
///
/// **Carries no environment and no command line** — FR-005a and SC-025 forbid a task's
/// environment reaching any log or crash report, and an error is the most natural place for one
/// to leak. See guarantee T10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnFailure {
    /// `command[0]` was not found on `PATH`, or is not executable.
    NotExecutable,
    /// The working directory was resolved and is gone, or is not a directory, by the time the
    /// child tried to enter it. A race, not a bug.
    CwdUnusable,
    /// A terminal or a pipe could not be allocated — descriptors exhausted, or no pty free.
    NoDevice,
    /// A limit in `ResourceLimits` was refused by the kernel. Distinct from the process later
    /// exceeding one, which is an ordinary death (guarantee T8).
    LimitRefused,
    /// Anything else the kernel said.
    Failed(std::io::ErrorKind),
}

/// Why a control operation did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlError {
    /// The process has ended. **Not an error the caller surfaces**: a notification has no
    /// response (§4.2) and FR-019 makes terminating an exited task a success.
    Gone,
    Failed(std::io::ErrorKind),
}
```

**Every `SpawnFailure` becomes `-32011` on the wire**, because FR-004 and SC-015 admit exactly one
outcome for a command that could not be started and §4.4 gives that outcome one code. The variant
survives as the `data.reason` §4.4 requires for an error a user can act on — and the distinction
the client needs from it is **whose fault it is**: `NotExecutable` and `CwdUnusable` are the
developer's to fix and are what §4.4's wording enumerates, while `NoDevice` and `LimitRefused` are
the instance's condition and want different words in front of a human. Neither reason may carry
the environment (guarantee T10, FR-005a, SC-025).

### Guarantees

| # | Guarantee | Requirement |
|---|---|---|
| T1 | The port cannot be given an unchecked working directory. `ResolvedPath` has no constructor but `resolve` | §4.7, FR-003, Principle VI |
| T2 | Spawning either yields a running process or a `SpawnFailure`. There is no third outcome, and no shape in which a failure becomes a task that started and exited | FR-004, SC-015 |
| T3 | With `Shape::Pty`, `read` yields `Stream::Stdout` and never `Stream::Stderr`. The merge happened in the kernel; the port does not choose it and cannot undo it | FR-008, A-TASKSTREAM, SC-028 |
| T4 | `Ended` is returned only after every byte the process wrote has been returned. A process that exits with bytes in the buffer yields those bytes first | FR-022, SC-011 |
| T5 | Bytes are returned exactly as written — no translation, no normalisation, no UTF-8 validation. The port has no opinion about what a byte means | FR-009, SC-003 |
| T6 | `read` never blocks longer than `timeout`. A chunk due at the 20 ms time bound is flushed at 20 ms, whatever the process is doing | FR-011, SC-001, plan.md *Chunk time bound* |
| T7 | Not reading is the only backpressure. The port never buffers on the caller's behalf, never drops, and never signals the producer to slow down | FR-013, SC-021 |
| T8 | `signal` reaches the process group, so every descendant receives it at any depth | FR-006a, FR-018, SC-027 |
| T9 | `reap` is idempotent and non-blocking, and reports `Exit::Signal` for a signal death rather than an encoded code — carrying **whichever** signal the kernel delivered, including ones this feature never sends | FR-020, FR-021, SC-010 |
| T10 | Nothing the port returns, logs or formats carries the environment. `SpawnRequest`'s `Debug` redacts `env`; `SpawnFailure` has no field it could occupy | FR-005a, SC-025 |
| T11 | The port makes no delivery decision. It reports what the process wrote; what reaches the wire is decided above it | FR-010, FR-011, Principle VIII |
| T12 | `TaskOutput` is `Send` and not `Sync`; `TaskControl` is `Send + Sync` — see below | FR-012, SC-006 |
| T13 | A `signal` issued while this task's reader thread is blocked in `read` takes effect without waiting for that read to return. This is what bounds `workspace/close` at plan.md's five seconds whatever the process is doing, including a process blocked in `write` against a full retention buffer | FR-024, SC-013, FR-012 |

**T12 is the shape the whole feature rests on.** The obvious port — one trait with `&mut self`
and a task handle on every method — needs a mutex around the runner, and a keystroke would then
wait behind a read that is blocked because the process has nothing to say. That is FR-012's
failure produced by the port's own shape, and SC-006 would measure it. Splitting the task into a
reading half owned by its thread and a controlling half shared with dispatch means a write never
waits for a read. It is the same reasoning F004's `FileWatcher` used to justify `Send` without
`Sync`, applied to a port with two callers instead of one.

---

## Resource limits — what they bound, and what they do not

```rust
/// Per-process ceilings, applied between fork and exec so the child is already constrained when
/// it starts, and inherited by everything it spawns (FR-006).
///
/// **The values are plan.md's, and there are two of them.** FR-006b requires "stated quantities
/// fixed in the plan, not judgements made per task"; plan.md's *Fixed Quantities* table now
/// states them, and this type carries exactly the limits it states and no field for a limit it
/// deliberately declined to set. `FIXED` is what every task gets — the struct exists so a test
/// can construct another and provoke `SpawnFailure::LimitRefused`, not so a caller can tune one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Address space a single process may map, set as **both the soft and the hard limit**.
    /// The ceiling A-TASKLIMIT's realistic runaway hits.
    ///
    /// Both, because a child may raise its own soft limit up to its hard limit: a soft-only
    /// ceiling is one the process being bounded can simply remove, and the removal is invisible
    /// until the first measurement of a runaway finds it was never bounded.
    pub address_space_bytes: u64,
    /// Core dump size. **Zero, and required to be** — FR-005a forbids a task's environment
    /// reaching any log or crash report, and a core dump is a crash report carrying the whole
    /// environment. This is a requirement satisfied by a limit, not a limit chosen for tuning,
    /// and it is the one field here that may not be varied by a caller for any reason.
    pub core_bytes: u64,
}

impl ResourceLimits {
    /// plan.md's *Fixed Quantities*: 16 GiB of address space, soft and hard; no core dumps.
    pub const FIXED: Self = Self {
        address_space_bytes: 16 * 1024 * 1024 * 1024,
        core_bytes: 0,
    };
}
```

**What they catch** (A-TASKLIMIT): the single runaway — a test with an allocation bug, a
development server that leaks, a tool that never frees. The process is **refused further address
space at its ceiling**, in seconds and without anything else noticing, which is what SC-026
measures: the allocation **denied** within 2 seconds of being requested, and the engine surviving
in 100% of exercised cases. The observable is the denied allocation and its timestamp, **reported
by the fixture**, not an exit notification. `RLIMIT_AS` fails an allocation and kills nothing;
what the process does next — abort, retry, carry on — is the program's own policy, so measuring to
an exit would measure `fixture_alloc`'s abort policy and would fail a correct implementation whose
task happens to handle `ENOMEM`. The purpose the limit serves is that a runaway cannot take the
instance down, not that it dies; the mechanism that guarantees a kill is a cgroup, which
A-TASKLIMIT records as unavailable until F005.

**What they do not catch, stated because a reader will otherwise assume they do:**

- **A process tree that collectively exhausts the instance.** Sixty-four compilers at three
  gigabytes each is a hundred and ninety-two, and no single limit was exceeded. This is recorded
  and accepted (A-TASKLIMIT), not closed. Closing it is cgroup delegation, which depends on
  provisioning F005 has not specified, and it belongs to whichever feature builds the shared
  supervisor §7.3 describes.
- **CPU time.** Not limited, deliberately (plan.md). `RLIMIT_CPU` counts per process, and any
  value low enough to catch a spinning process is low enough to kill a real compile. The runaway
  that takes an instance down is memory; a process spinning on CPU stays visible in
  `execution/list` and stoppable through `execution/terminate`.
- **Process count.** Not limited, and the reasoning is plan.md's rather than this contract's:
  `RLIMIT_NPROC` is **per user, not per process**, and under A-EC2 the engine runs as the same
  user as every task it starts, so setting it for a task bounds the developer's entire session
  including the engine. A limit that can starve the engine is not a limit that protects it. A
  fork bomb remains a tree problem, owed with the rest.
- **Disk, at any granularity.** `RLIMIT_FSIZE` is not set, because plan.md's *File size* row
  **declines** it rather than fixing a value — the limit caps a **single file** and a build
  legitimately writes large ones, so any value low enough to stop a runaway log breaks real
  output — and FR-006b forbids this document inventing one where the plan has not. So neither a
  single enormous file nor a tree writing ten thousand small ones is bounded, and a task can fill
  the volume. §5.5 and §16 accept an unquota'd disk under single tenancy — "a developer filling
  their own disk" — which covers the consequence.
- **Anything about the pseudo-terminal's kernel buffer.** That buffer is what slows a producer
  when nobody reads (T7, research.md, *Backpressure comes for free*), and no limit here changes
  its size or its behaviour.
- **The engine.** These limits are the child's. Nothing here bounds the engine's own memory, and
  A-LSP's threat — the out-of-memory killer choosing the engine — is mitigated by the child dying
  first, not by the engine being protected.

---

## What `execution/list` and `workspace/close` need

Both rows were added to §4.8 on 2026-09-24. **Neither adds anything to this port**, and the reason
is worth stating rather than leaving as a silence a reader would mistake for an omission.

**`execution/list` needs nothing.** Enumeration is over the `TaskSet` the application already
holds (data-model.md), keyed by `TaskId` — an identity this port has never heard of. Every field
of a list entry is application state: `taskId`, `workspaceId`, `command` and `pty` are recorded at
`runTask`, `retained` is the bounded buffer the application owns, `pid` came back from `spawn`,
and `running` is `Task::state`, which the reader thread has already updated from `ReadOutcome` and
`reap`. **The listing therefore touches the port zero times**, which is what makes
`execution/list` a pure read (task-methods.md, guarantee 2) and what makes its snapshot semantics
honest: it reports what the engine has observed, not what the kernel currently holds
(task-methods.md, guarantee 3). A port method that probed liveness per task would replace one
in-memory read with N syscalls and would still be a snapshot by the time it was serialised.

**`workspace/close` needs only `TaskControl::signal`, which it already has.** Selecting the
workspace's tasks is `TaskSet::drain_for_workspace` — application state, because the port takes a
resolved working directory and does not know what a workspace is. Ending them is `signal(Term)`,
then `signal(Kill)` after 5 s measured on the `Clock` port F004 added. Reporting them is the
ordinary exit path: `reap`, then `onExit` (task-events.md, guarantee 16).

**The one property this depends on is T12, and it is not free.** Closing a workspace arrives on
the dispatch thread while every one of that workspace's tasks has a reader thread blocked in
`read`. `TaskControl` being `Send + Sync` behind an `Arc`, separate from the `TaskOutput` its
thread owns, is what lets the close signal all of them immediately (T13). Under the obvious port
— one trait, `&mut self`, a task handle per call — a close would have to wait for each blocked
read to return before it could signal that task, and a task blocked because its retention buffer
is full is a task whose read will not return until somebody drains it. The bound plan.md fixes at
five seconds would have been unbounded, and SC-013 would have been unmeetable for exactly the
tasks it most needs to end. The split was justified for a keystroke (T12); it is load-bearing for
a close.

---

## The fake — why it is part of the contract

F003 established that an in-memory fake is a contract obligation rather than a test helper, and
F004 gave the reason that applies here with more force: **the adapter is Linux-only by
construction**, and a real pseudo-terminal cannot be driven deterministically. A build that outruns
a link, a process that exits with bytes still buffered, and a kernel refusing a limit are all cases
a real process produces rarely and a test must produce every run.

`FakeRunner` implements `TaskRunner` with no process at all, and must reproduce:

- **A scripted output sequence per task**, of `(Stream, Vec<u8>)` and `Idle`, handed out one
  `read` at a time. Chunking, ordering and the frame-size bound are then exercised against exact
  inputs (FR-010, FR-011, SC-004, SC-005) — including a scripted 4 MiB run with no newline in it,
  which is SC-005 as a unit test.
- **`Ended` after the scripted bytes and not before**, with a case that scripts bytes *after* the
  process is marked exited, so T4 and FR-022 are tested rather than assumed. This is the spec's
  *Output arriving after the process has exited* edge case, and it is unreachable on demand with a
  real process.
- **A settable exit**, `Exit::Code` or `Exit::Signal`, so SC-010's "distinguishable in 100% of
  cases" is a unit test and does not require killing anything — including a signal this feature
  never sends, `SIGSEGV`, which is the case a closed `Exit::Signal` would have made unrepresentable
  and which a real process obliges only by crashing.
- **A `read` that yields nothing forever**, so the retention bound and the decision to stop
  reading are exercised (FR-013, FR-013a, SC-021) without a process capable of outrunning a link.
- **A recorded stdin buffer.** SC-007 asserts bytes reach the process byte-for-byte; the fake is
  where "the process" can be inspected.
- **A record of every resize and every signal**, with the signal recorded against the group rather
  than the pid, so T8 and SC-027 are assertable. A fake that records only that `signal` was called
  tests half of FR-018.
- **`SpawnFailure` on demand**, every variant. FR-004 and SC-015 live entirely in the failure
  path, and a fake that cannot fail tests only the case where nothing goes wrong.
- **A `signal` that lands while a `read` is outstanding**, so T13 is asserted rather than assumed
  — the fake's `read` can be told to block until released, which is how `workspace/close` against
  a task whose buffer is full (SC-013) becomes a unit test instead of a race nobody can provoke.
- **`Shape::Pty` yielding only `Stream::Stdout`.** SC-028's first half is then a unit test; its
  second half — `isatty` reporting true — is not, and needs a real process (below).

`FakeClock` is F004's, moved from `Cell<Millis>` to a `Mutex` plus a `Condvar` so it can be
shared with the thread doing the waiting — a `Cell`-backed fake is actively `!Sync`. The
chunker's time bound is driven by `advance(Millis)` and
never by elapsed real time, and `FakeRunner::read` advances it by its `timeout` when the script is
empty — the same coupling F004's `FakeWatcher` established, and for the same reason: without it
every volume test is wall-clock-dependent and slow, and a slow test is a test that gets marked
flaky and then ignored.

Both live beside the tests, not in `src/`, so neither can be wired into a real composition by
accident — F003's rule for `FakeWorkspace`.

**What the fake cannot prove, and what must therefore spawn a real process.** Whether a process
believes it is attached to a terminal (FR-002, US2 scenario 4), whether it observes a resize
(FR-016, SC-009), whether `0x03` becomes a signal rather than a byte (FR-015, SC-008), and whether
a signal reaches a grandchild (FR-018, SC-027) are all properties of the mechanism, not of any
decision. Those tests spawn a process **locally**: no remote host and no network (A-TEST, FR-033,
SC-017). Everything else — chunking, ordering, backpressure, retention, identity lifetime,
refusing a live identity, reporting a start failure — is decided above the port and is tested
against the fake.

---

## What is NOT behind this port

**The wire.** The reader thread serialises notifications itself and writes them straight to the
`FrameWriter` F004 built (`engine/src/adapters/outbound/frame_writer.rs`) on its task's own
thread, taking the bulk entry point; the writer holds the output mutex for exactly one frame.
Nothing queues between the two, which is what keeps a blocked write reaching back to the process
(FR-013) and keeps a task's `onExit` behind the output it follows (FR-022) — one thread writes
both, in order. The port yields bytes; `task_threads.rs` turns chunks into frames. What that path
must not do is hold the mutex for longer than one frame, and — the half F004's seam does not
provide — it must not let a task's output take its turn ahead of a completion response: §4.6
requires interactive traffic to win the race to the wire **in both directions**, and a mutex
orders by acquisition. The engine's answer is `FrameWriter`'s fairness gate, a bulk writer that
yields while an interactive writer is waiting (design.md **[CONFLICT 9]**). **That is a
measurement obligation under Principle V, not a comment** — and F010 is the first feature to put
real volume through the seam, so SC-006 measures the gate and the writer as much as the runner.

**The session and the identity.** `TaskId`, the live set, refusing an identity that is already
running (FR-031c), releasing one whose exit has been delivered (FR-023) and knowing which
attachment is owed what (FR-031b) are domain and use-case concerns. The port knows about a process;
it has never heard of a task id.

**A workspace.** The port takes a resolved working directory and nothing else. It does not know
which workspace a task belongs to, which is why FR-024's "closing a workspace terminates its
tasks" is a use-case rule applied through `TaskControl::signal` rather than anything the port
offers. §4.8 gained `workspace/close` on 2026-09-24, so the frame that triggers that rule now
exists; what it needs from the port did not change, and *What `execution/list` and
`workspace/close` need* above says why.

**The client.** `LocalTaskProvider` returns `Unsupported` for every task method in v1, and no
local pseudo-terminal ships. §13.2 was amended on 2026-09-24 to say so in the system specification:
local tasks are "built by F015 `local-mode`, not by F010". The degradation is specified rather than
missing, exactly as A-WATCHLOCAL specified F004's. The port is the seam that makes a future local
runner a new adapter rather than a change.

**A second implementation for another platform.** There is one adapter, for Linux, because the
engine runs on Linux and F004's watcher already assumes it. The port exists for testability and
for Principle VIII, not for portability nothing has asked for — and `portable-pty`, which §13.2
names, is F015's dependency in a different binary with a genuine cross-platform need
(research.md, *The pseudo-terminal mechanism*).
