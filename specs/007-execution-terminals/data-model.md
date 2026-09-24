# Data Model: Execution Terminals

**Branch**: `feature/F010-execution-terminals` | **Date**: 2026-09-24 | **Plan**: [plan.md](./plan.md)

Two locations, one vocabulary, and **no third**. **Engine memory** holds every task, its bytes and
its ending, and dies with the process. **The wire** carries the eight `execution/*` messages §4.8
now defines. There is no database, no file and no schema change: F004 took the client projection
from v1 to v2, and this feature takes it nowhere, because nothing here outlives the engine.

Rationale lives in [research.md](./research.md) and is not repeated here. Where a decision has a
source it is linked by heading; this document owns shape, not argument. Every entity names the
requirements that create it.

**Nothing in this document is `async`.** The engine adds no runtime (plan.md, Technical Context),
so every type here is an ordinary value moved between threads, and every port is a synchronous
trait. MSRV is 1.75.

---

## Engine-side entities

### `TaskId`

The identity of one task, **chosen by the client** (§4.8, FR-001). This is the single fact the
rest of the feature is built on: because the client mints it, a client that reconnects already
knows what to ask for, and `execution/attach` is a lookup rather than a discovery protocol
(A-TASKLIFE). An engine-assigned id would make "already running" a question the client could not
ask, which is the reversal condition research.md records under *Attaching to a task that is
already running*.

| Field | Type | Rule |
|---|---|---|
| `0` | `String` | Opaque. Compared by exact match, never by prefix, and never parsed |

```rust
pub struct TaskId(pub String);   // + Display, as SessionId and WorkspaceId already have
```

| Rule | Source |
|---|---|
| Minted by the client, exactly as `WorkspaceId` is (A-WORKSPACE) | §4.8, FR-001 |
| Unique among **live** tasks. Starting under an id that is already live is refused, and does not start a second process | FR-031c, SC-022 |
| Reusable once the task has been released — that is, once it has exited *and* its output has been delivered | FR-023 |
| Untrusted input, like every value off the wire. It is a map key and nothing else: it never reaches a path, a command line or an `exec` | Principle VI, §4.7 |
| Safe to log. The task's `env` is not (FR-005a), so the id is the correlation handle diagnostics use instead | FR-005a, SC-025, A-OBS |

**Scope is the whole engine, not one workspace, and §4.8 forces that reading.** `runTask` and
`attach` carry `workspaceId` and `taskId`; `writeStdin`, `resizePty`, `terminate`, `onStdout`,
`onStderr` and `onExit` carry `taskId` alone. A per-workspace id would leave those six unable to
resolve a task at all. So the id is engine-unique, and the owning workspace is a **property of the
task** rather than part of its key — which is what FR-024 needs, since closing a workspace must
terminate that workspace's tasks and not another's.

**What §4.8 does not state, and this document does not invent**: any syntactic constraint on the
id. No length, no character set, no collision rule between two clients of one engine. Under A-EC2
there is one developer per instance, so a collision is a client racing itself, which FR-031c
already refuses. A shared instance would make this a real question; A-WORKSPACE's reversal
condition is the same one.

### `Task`

One running command, and everything the engine must know to keep it running, reach it and end it.
FR-001, FR-002, FR-003, FR-005, FR-006, FR-006a.

| Field | Type | Rule |
|---|---|---|
| `id` | `TaskId` | Identity. Client-chosen; see above |
| `workspace` | `WorkspaceId` | Which workspace owns it. Set at `runTask` and never changed — it is what makes FR-024 a lookup rather than a search |
| `command` | `Vec<String>` | Non-empty. `[0]` is the programme, the remainder are its arguments verbatim. **Argv, not a command line** — see below |
| `cwd` | `ResolvedPath` | Proven a descendant of the workspace's `CanonicalRoot` by the existing two-stage resolve. Defaults to the root when the caller names none |
| `env` | `BTreeMap<String, String>` | Overrides, applied **over** the environment the engine inherited (spec Assumptions). Ordered for reproducible spawning, not because order means anything |
| `pty` | `bool` | Fixed at start. Decides both `isatty` and the stream shape, and the two cannot be chosen separately (A-TASKSTREAM) |
| `pid` | `i32` | The child's process id, returned by `runTask` and `attach`. Known only once the spawn has succeeded |
| `pgid` | `i32` | The task's own process group. Equal to `pid` by construction, because the child is made leader of a new group (FR-006a) |
| `limits` | `ResourceLimits` | Applied to the child, inherited by its children (FR-006) |
| `state` | `TaskState` | `Running`, or a terminal `ExitStatus`. See *State transitions* |
| `retained` | `RetainedOutput` | The bounded buffer, and after the end, the terminal status awaiting delivery |
| `attached` | `bool` | Whether a client is currently receiving this task's output. Orthogonal to `state` |

**`command` is argv because the specification forbids the alternative.** §4.8 writes `command` and
does not say whether it is one string the engine splits or a vector it executes. The difference is
whether the engine implements word splitting, quoting and expansion — that is, whether the engine
contains a shell — and spec.md's *What this feature is not* says plainly: "**Not a shell.** A
login shell is a command like any other; this feature does not implement one, configure one, or
assume one." A vector is the only reading consistent with that. The developer-facing terminal that
spec.md's Assumptions describe is then an ordinary task whose argv names a shell, which is exactly
what that assumption says it is. **§4.8 does not state this**, and it should; the arity of
`command` is recorded here as derived from a constraint, not chosen.

| Rule | Source |
|---|---|
| `cwd` off the wire is untrusted, resolved by the engine independently of the client, and refused with `-32002` if it escapes the root | FR-003, §4.7, Principle VI |
| Runs as the engine's own user, with no escalation and no set-uid path | FR-005, A-SEC, A-EC2 |
| `env` is never written to a log line or a crash report, including when the spawn fails | FR-005a, SC-025, A-OBS |
| Its own process group, so terminating it reaches everything it spawned | FR-006a, FR-018, SC-027 |
| A spawn that fails produces **no** `Task` at all; the `runTask` request fails instead | FR-004, SC-015 |

**Lifetime.** Created by `execution/runTask`, and only by a spawn that succeeded. Ends when the
process ends — by its own exit, by a signal, or by `execution/terminate`. Released, and its id
freed, once the end has been reported *and* the retained output delivered (FR-023). It survives
the client's connection dropping (FR-031, A-TASKLIFE) and does not survive the workspace closing
(FR-024), the engine exiting (FR-025), or the engine process being replaced (see *What is not
persisted*).

**`env` is the one field with a handling rule attached to it.** FR-005a and SC-025 make it
unloggable, which means the `Task` cannot derive `Debug` naively: a `#[derive(Debug)]` on this
struct plus one `tracing` call carrying `?task` puts every variable in the log, and SC-025 asserts
zero. `Debug` must be written by hand to elide `env`, or `env` wrapped in a type whose `Debug`
elides itself. The second is harder to defeat by accident and is the shape this document
recommends; the choice belongs in design.md.

**One observation, recorded rather than resolved.** FR-005a redacts `env` and says nothing about
`command`, which carries credentials about as often — a token in a `curl` argument is not
protected by a rule about environment variables. SC-025 measures only the environment. Widening it
is a specification change, not a design decision, so it is flagged and not taken here.

### `TaskSet`

The engine's live tasks. One per engine, **not one per workspace**, for the reason `TaskId` gives:
six of the eight methods address a task by id alone.

| Field | Type | Rule |
|---|---|---|
| `tasks` | `BTreeMap<TaskId, Task>` | Ordered for determinism in tests and listings; nothing depends on the order |

| Operation | Effect | Behaviour on repeat |
|---|---|---|
| `start(task)` | Inserts a task the runner has already spawned | **Refuses** an id that is already present (FR-031c) |
| `get(id)` | The task, live or ended-and-undelivered | — |
| `release(id)` | Removes a task that has ended and whose output is delivered | Releasing an absent id changes nothing and is not an error |
| `drain_for_workspace(ws)` | Every task of one workspace, for termination on close | Draining twice yields nothing the second time |
| `drain_all()` | Every task, for engine exit | As above |

**It is deliberately *not* idempotent where `WatchSet` is, and the contrast is the requirement.**
F004's `WatchSet::acquire` called twice yields one watch with two reasons, because asking to watch
something already watched is a client reconciling its set. `TaskSet::start` called twice is a
refusal, because the second call would fork a second process, and FR-031c exists to say that a
client racing its own reconnection must not do that silently. One data structure is convergent,
the other is not, and each is right for what it holds.

**It outlives a connection.** This is the whole of A-TASKLIFE expressed as a lifetime. The set is
not indexed by, scoped to, or cleared with the transport: a dropped connection sets `attached`
false on the tasks that had a viewer and changes nothing else. Nothing in this type knows what a
connection is.

| Invariant | Source |
|---|---|
| An id present in the set is live or awaiting delivery; an id absent from it is free | FR-023 |
| After a hundred start-and-exit cycles the set returns to its starting size, and so does the count of running processes | SC-014 |
| Closing a workspace empties that workspace's share of the set and terminates each member | FR-024, SC-013 |
| No member survives the engine process | FR-025, plan.md Storage |

**Lifetime.** Engine memory, beside the workspace registry (`WorkspaceRoots`, "in memory, for the
engine's lifetime") and F004's `WatchSet`. It dies with the process. See *What is not persisted*
for what that costs, which is more than it costs those two.

### `OutputChunk`

A portion of what one task has written. **Bytes, never text** (FR-009, SC-003, spec Key Entities).

| Field | Type | Rule |
|---|---|---|
| `task` | `TaskId` | Which task produced it |
| `stream` | `OutputStream` | `Stdout` or `Stderr`. When the task has a pty, **only `Stdout` is ever produced** (A-TASKSTREAM) |
| `bytes` | `Vec<u8>` | Exactly what the process wrote, in the order it wrote it. No decoding, no normalisation, no line-ending translation |

```rust
pub enum OutputStream { Stdout, Stderr }
```

| Rule | Source |
|---|---|
| Emitted as it is produced, never accumulated until the process ends | FR-007, SC-001 |
| Byte-for-byte what was written, including ANSI escapes and invalid UTF-8 | FR-009, SC-003 |
| Ordered within its task, with no ordering promised *between* tasks | FR-010, SC-004 |
| Bounded so no frame exceeds §4.1's cap, including for output containing no line break | FR-011, SC-005 |
| Emitted on a size bound **or** a time bound, whichever comes first | research.md, *Chunking, ordering, and what is pure* |

**A chunk carries no sequence number, and ordering is structural rather than checkable.** One
reader thread owns one task's descriptor and one `FrameWriter` holds the lock for exactly one
frame, so chunks reach the wire in the order they were read and the transport preserves it. That
satisfies FR-010 and SC-004. The cost is worth naming: a client cannot *detect* a lost or
reordered chunk, because there is nothing to compare. This is acceptable for the same reason A-REQ
is — an in-flight request dies with its connection, and a connection that loses bytes without
dying is not a failure mode this transport has — but it does mean FR-031b's "in order, before
anything produced since" is a property of how the engine emits on reattachment, verifiable at the
engine and not at the client.

**The chunk's size bound is not stated anywhere it can be read.** Two ceilings are known and
neither is the value: §4.1 caps a frame at 1 MiB, and A-BULKSIZE fixes 512 KiB as the largest
**raw** payload one frame may carry, precisely because base64 is four bytes out for every three in.
So whatever the plan fixes must be at or under 512 KiB raw, and SC-005's 4 MiB unbroken line is
therefore at least eight chunks. plan.md fixes no number. See *Quantities the plan owes*.

**Lifetime.** Between the reader thread's read and the frame writer's flush, plus however long it
waits in `RetainedOutput`. An `OutputChunk` is never persisted and never reaches a file.

### `ExitStatus`

How a task ended. **Distinct states, not one field with a convention** — that phrasing is spec.md's
own, in Key Entities, and FR-021 makes it testable.

```rust
pub enum ExitStatus {
    /// The process ran to completion and returned this code.
    Exited { code: i32 },
    /// The process was killed. `signal` is the number that killed it.
    Signalled { signal: i32 },
}
```

| Rule | Source |
|---|---|
| Exactly one variant per task, constructed once, terminal | FR-020, FR-021 |
| A signal death is distinguishable from an exit in every exercised case | FR-021, SC-010 |
| Delivered **after** every chunk the process produced before ending | FR-022, SC-011 |
| Never constructed for a command that could not be started — that is a failed request, not an ending | FR-004, SC-015 |
| Reportable to a client that was absent when it happened | FR-031b, SC-020 |

**The wire cannot carry this enum as an enum, and §4.8 is the reason.** `execution/onExit`'s
catalogued payload is `taskId`, `exitCode`, `signal?` — one code plus an optional signal, which is
the convention Key Entities rejects. The domain holds the enum; the boundary maps it. The mapping
rule the client must apply is the conservative one, in the shape `RefusalReason` already uses:
**`signal` present means `Signalled`, whatever `exitCode` says.** A client that tests `exitCode`
first will read a signalled death as an ordinary non-zero exit, which is precisely the confusion
FR-021 exists to prevent.

**What `exitCode` carries when the process was signalled is undetermined.** §4.8 does not say, and
neither spec.md nor research.md addresses it. The two live candidates are the shell convention
(`128 + signal`) and a value the client is told to ignore. This document states the invariant the
client depends on — read `signal` first — and declines to invent the other half. It belongs in
contracts/task-events.md.

**Lifetime.** Created once when the process is reaped, held in `RetainedOutput` until delivered,
and gone when the task is released.

### `RetainedOutput`

What the engine is holding for a client that is not currently taking it. Bounded, in memory, and
the same structure whether a client is attached or not. FR-013, FR-013a, FR-031a, FR-031b.

| Field | Type | Rule |
|---|---|---|
| `chunks` | `VecDeque<OutputChunk>` | FIFO. Chunks rather than a flat byte buffer, so a `pty: false` task's two streams stay separable across a detachment |
| `bytes_held` | `usize` | The bound is on **total bytes**, not on the number of chunks, because a chunk count bounds nothing when a chunk may be half a megabyte |
| `ending` | `Option<ExitStatus>` | Set when the process is reaped. This is what makes FR-023's "released once it has exited **and** its output has been delivered" implementable, and SC-020 satisfiable |

| Rule | Source |
|---|---|
| Bounded by a stated quantity, so memory held for one task is a number somebody chose | FR-013a |
| At the bound, the reader **stops reading** the task's descriptor. Nothing is dropped, nothing is marked, nothing is announced | FR-013, research.md, *Backpressure comes for free* |
| The bound and the behaviour are identical while detached. A process must not discover it is unobserved by being treated differently | FR-031a, SC-021 |
| Drained in order on delivery; on reattachment everything retained goes before anything produced since | FR-031b, SC-019 |
| Holds the ending until it is delivered, so a task that ended unobserved can still say how | FR-031b, SC-020 |

**Backpressure is invisible on the wire, and that is worth stating once.** There is no
notification for "this task is being slowed", and there is none for the process either — it
observes a write that blocks, which is what a process writing to an unread terminal already
observes. So a build that is being slowed by a saturated link and a build that is simply slow look
identical to the developer. FR-013 asks for the behaviour and nothing asks for the signal; making
it visible would be a new notification and a new requirement.

**Its bound is a plan value that plan.md does not state.** FR-013a says so in as many words —
"a stated quantity fixed in the plan, not a judgement made per task" — and the plan states none.
See *Quantities the plan owes*.

**Lifetime.** With its `Task`. It is memory, never a file (plan.md, Storage), and it dies with the
process.

### `ResourceLimits`

The per-process ceilings a task runs under. FR-006, FR-006b, A-TASKLIMIT.

| Field | Type | Rule |
|---|---|---|
| `memory` | `u64` | Bytes. The one SC-026 makes mandatory: a process exceeding it is terminated within 2 seconds and the engine survives |
| *(further ceilings)* | — | **The set is not fixed by any artefact.** plan.md names the mechanism (`nix`'s `resource` feature) and no member but the memory one that SC-026 implies |

| Rule | Source |
|---|---|
| Applied to the task's own process, between the fork and the exec | FR-006, A-TASKLIMIT |
| Inherited by everything the task spawns, at any depth | FR-006, A-TASKLIMIT |
| Stated quantities fixed in the plan, not judgements made per task — "a limit nobody chose is one nobody can defend when it fires" | FR-006b |
| Identical for every task. Nothing in `runTask`'s parameters varies them, and §4.8 offers no field that could | §4.8, FR-006b |

**Inheritance is a property of the mechanism, not machinery this feature writes.** Resource limits
set on a process are carried across `fork` and preserved across `exec`, which is what makes
"inherited by its children" true without anything walking a tree. One consequence follows and is
load-bearing: a limit has a **soft** and a **hard** value, and a process may raise its own soft
limit up to the hard one. Setting only the soft limit therefore makes FR-006's constraint
advisory — a task that wants more memory can simply take it. Both must be set. Nothing in spec.md,
plan.md or research.md says so, and this is the kind of gap that is invisible until a runaway
process is measured and found to have been unbounded all along.

**What this does not bound, said plainly: a tree.** A-TASKLIMIT records it as accepted rather than
solved. Sixty-four compilers at three gigabytes each is a hundred and ninety-two gigabytes on a
hundred-and-twenty-eight-gigabyte instance, with **every single process under its own ceiling and
no limit exceeded**. The per-process limit catches the realistic runaway — one process with an
allocation bug — and cannot see the aggregate. Reaching the aggregate takes deliberate
over-parallelisation, since `-j$(nproc)` on this instance is sixteen, so the gap is recorded and
left to whichever feature builds the shared supervisor (§7.3, F007).

Three further things it does not bound, for completeness, because each is a question somebody will
ask of this type:

- **The number of concurrent tasks.** There is no count limit; spec.md's Assumptions say so and
  give the reason — the per-process ceilings are a real bound and a count is an arbitrary one.
- **Disk.** A task filling the workspace's filesystem is bounded by A-WORKSPACE's decision not to
  enforce a quota, and surfaces as an ordinary engine error.
- **Network and CPU over time.** Unless the plan's fixed set includes a CPU ceiling, which it does
  not currently state.

**Lifetime.** A constant of the engine build, not per-task state. It is listed as an entity
because it is a value the `TaskRunner` port must be handed and a value tests must be able to
shrink; it holds nothing that changes.

---

## Wire types

For `protocol/src/wire.rs`, matching §4.8's Execution rows exactly as they now stand.

**On the naming convention, checked rather than assumed.** §4.8 states it: field names are written
camelCase in the tables and the wire carries snake_case. `wire.rs` implements that by doing nothing
special — no params or result struct carries `#[serde(rename_all)]`, the Rust field names are
already snake_case, and the only renames in the file are `kind` → `type` (a Rust keyword
collision) and the lowercase variants of `EntryKind`. F010's types follow by naming their fields
snake_case and adding no attribute. `taskId` is `task_id`, `exitCode` is `exit_code`.

### Output is bytes, and a JSON wire cannot carry bytes

This is the single most consequential shape decision in the feature, so it is stated before the
types rather than inside them.

FR-009 and SC-003 require every byte a process wrote to arrive unmodified, "including non-UTF-8
sequences, with zero substitutions". JSON strings are Unicode. `serde_json` will not serialise a
Rust `String` containing invalid UTF-8 because a `String` cannot contain one, and the conversion
that would let it — `String::from_utf8_lossy` — substitutes U+FFFD, which SC-003 sets to zero.
**There is no text path.** `wire.rs` already reached this conclusion once, for
`ReadFileResult::content`: "There is no utf8 path: assuming text corrupts binary content silently,
and a method that sometimes returns text makes every caller branch on it."

So `data` on `onStdout`, `onStderr` and `writeStdin` is **base64 of the raw bytes**, carried in a
`String`.

**What that costs, precisely.** Base64 is four characters out for every three bytes in, a 33%
expansion before the JSON envelope. Three consequences follow, and all three are already
constraints elsewhere:

1. **A chunk's raw size must stay at or under A-BULKSIZE's 512 KiB**, not §4.1's 1 MiB. 512 KiB
   raw encodes to roughly 683 KiB and leaves room for the envelope; a threshold at the frame cap
   would encode past it and fail with `-32007` on the first frame that carried a path.
2. **Encode and decode are paid on the highest-volume traffic in the system.** §4.6 makes this one
   pipe and one queue, and F010 is, in plan.md's words, the largest producer the channel will ever
   carry. SC-006's 50 MiB becomes roughly 66.7 MiB of wire bytes, encoded once and decoded once.
   That cost is inside what FR-012 forbids delaying interactive traffic, and inside what SC-006
   measures.
3. **Nothing negotiates the encoding.** `workspace/readFile` carries an `encoding` field because a
   file may usefully be known to be text; process output never is, so these payloads carry no such
   field and the encoding is fixed. **§4.8 states neither the encoding nor its absence**, which is
   a fourth edit owed to the catalogue beyond the three research.md listed. Recorded, not taken.

### The types

```rust
// ---- execution/runTask ----
pub struct RunTaskParams {
    workspace_id: WorkspaceId,
    task_id:      TaskId,
    command:      Vec<String>,          // argv; [0] is the programme. Non-empty.
    cwd:          Option<String>,       // untrusted; the workspace root when absent
    env:          BTreeMap<String,String>, // serde(default); overrides over the inherited set
    pty:          bool,                 // A-TASKSTREAM: chooses isatty AND the stream shape
}
pub struct RunTaskResult { pid: i32 }

// ---- execution/attach ----
pub struct AttachParams { workspace_id: WorkspaceId, task_id: TaskId }
pub struct AttachResult {
    pid:      i32,
    running:  bool,
    retained: ???,                      // UNRESOLVED — see below
}

// ---- execution/writeStdin ----  (notification)
pub struct WriteStdinParams { task_id: TaskId, data: String }   // base64

// ---- execution/resizePty ----   (notification)
pub struct ResizePtyParams { task_id: TaskId, cols: u16, rows: u16 }

// ---- execution/terminate ----   (request)
pub struct TerminateParams { task_id: TaskId, signal: ??? }     // UNRESOLVED — see below
// result: empty

// ---- execution/onStdout, execution/onStderr ----  (notifications)
pub struct OutputParams { task_id: TaskId, data: String }       // base64

// ---- execution/onExit ----      (notification)
pub struct ExitParams { task_id: TaskId, exit_code: i32, signal: Option<i32> }
```

**One params type serves both output notifications, and the stream lives in the method name.**
§4.8 gives `onStdout` and `onStderr` identical payloads — `taskId`, `data` — so a second struct
would be a duplicate whose only purpose is to be named differently. The domain's `OutputChunk`
carries a `stream` field because the domain needs to decide; the wire does not, because the method
has already said.

**`pty: true` means `onStderr` carries nothing, and the types must document that rather than
express it.** A-TASKSTREAM makes the choice exclusive: with a pseudo-terminal the task has **one
device**, `isatty` is true on both descriptors, and everything the process writes arrives merged on
`onStdout`. `onStderr` is not "usually empty" and not "rarely used" — for a `pty: true` task it is
never emitted at all, and SC-028 asserts zero bytes on it. With `pty: false` the two are separate
pipes, both notifications are used, and the process is not attached to a terminal. The type system
cannot carry this: `OutputParams` is one struct used by two methods, and no field of it varies
with `pty`. It is therefore a **doc comment on `RunTaskParams::pty` and on `OutputParams`**, and an
invariant a test asserts, which is the honest place for a constraint that spans two messages.

**`resizePty` on a `pty: false` task has no defined behaviour.** There is no terminal to resize.
The candidates are a silently ignored notification and an error, and since `resizePty` is a
notification it cannot return one. §4.8 does not address it and neither does spec.md. Ignoring is
the only thing a notification can do; whether that is right is contracts' call.

**`cols` and `rows` are `u16`, non-zero.** The width and height of a terminal are unsigned shorts
in the structure the kernel takes, so a wider type would only widen the range of values that must
be rejected. Zero is meaningless and some programmes divide by it. §4.8 states neither the type nor
the constraint; both are derived here and belong in contracts/task-methods.md.

### Two fields §4.8 names and does not define

**`AttachResult::retained` has no stated type, and the obvious one breaks the frame cap.** §4.8's
row is `{pid, running, retained}`. research.md, *Attaching to a task that is already running*,
describes the call as returning "the task's current state and the output retained since it was last
read", which reads as the bytes themselves. If it is the bytes, then a retention bound large enough
to be useful — SC-024 exercises a task producing 50 MiB — cannot fit one frame, since §4.1 caps at
1 MiB and A-BULKSIZE's inline ceiling is 512 KiB raw. The alternative shape is a **count**, with
the retained bytes replayed as ordinary `onStdout` notifications immediately after the response,
which satisfies FR-031b's ordering requirement and reuses the chunking that already exists.

This document states the constraint and **declines to choose**: the choice is a contract decision
with a wire consequence, and it belongs in contracts/task-methods.md. What is certain is that
`retained` cannot be an unbounded inline payload.

**`AttachResult` has nowhere to put an ending, and SC-020 requires one.** FR-031b and SC-020 say a
task that exited while nobody was attached must have its exit reported to the client that
reattaches. `{pid, running, retained}` carries `running: false` and no code and no signal. Either
`onExit` is re-emitted after the attach response — which nothing states — or the result needs the
status. **This is a genuine hole in the amended §4.8 row**, and it is the same class of absence
that produced the row in the first place.

**`TerminateParams::signal` has no stated type either.** §4.8 names the parameter; FR-017 requires
the request to state which signal it sends; spec.md's Assumptions place "the specific signals an
interrupt and a stop request send, [and] the escalation after a process ignores one" among the
plan-level decisions — and plan.md decides neither. A name (`"SIGTERM"`) and a number (`15`) are
both defensible and they are not interchangeable across a protocol boundary. Undetermined; see
*Quantities the plan owes*.

### Error codes

`wire.rs`'s `codes` module holds six constants and its own rule: "neither may write the integer
inline — a literal `-32001` in a match arm is a fact stated twice". F010 needs one that exists in
§4.4 and is missing from the module, and one that exists nowhere.

| Code | Meaning | Status |
|---|---|---|
| `-32006` | Task not found or already exited | **In §4.4, absent from `codes`.** F010 adds `TASK_NOT_FOUND` |
| `-32001` | Workspace not registered | Already present. `runTask` and `attach` resolve a workspace |
| `-32002` | `cwd` escapes the workspace root | Already present. FR-003 |
| `-32009` | Workspace registered, root gone | Already present |
| *(none)* | **Task id is already live** | **No code exists.** FR-031c and SC-022 require the refusal |

That last row is a gap, not a choice. FR-031c requires `runTask` under a live identity to be
refused; `-32006` is its exact opposite and cannot be reused without making "not found" and
"already there" the same answer. `-32602` (invalid params) would technically carry it and would
make a client's recovery — attach instead of start — indistinguishable from a malformed request.
A seventh application code is owed to §4.4. This document does not mint one.

**`-32006` also answers two questions with one code, and that matters after a restart.** "Not
found" and "already exited" are the same value, so a client that reattaches to a remembered id and
receives `-32006` cannot tell a task that finished and was released from a task the engine forgot
when it restarted. The first means "your build finished while you were away"; the second means
"your build may still be running and I cannot reach it". See *What is not persisted*.

**Neither the added method nor the added codes increment `protocolVersion`.** A-PROTOVER and §4.8
are explicit that adding a method does not. What tells a client whether the engine has them is
`auth/handshake`'s `capabilities`, whose tokens are method names matched exactly — an engine
without `execution/attach` in its set is one a client must not assume it can reattach to.

---

## State transitions

### A task

Two axes, and they are independent. That independence is A-TASKLIFE stated as a shape: whether a
client is watching has no bearing on whether the process is running.

```
                          execution/runTask
                                 │
                                 ▼
                           ┌──────────┐
                           │ Starting │───── spawn fails (FR-004) ──▶ no Task; the REQUEST fails
                           └────┬─────┘                               (never an onExit — SC-015)
                       spawned; │ pid and pgid known
                                ▼
                           ┌─────────┐◀──── writeStdin / resizePty ────┐
                           │ Running │                                 │
                           └────┬────┘──── retention bound reached ────┘
                                │             (reader stops; the process blocks)
              ┌─────────────────┼──────────────────┐
     process exits     killed by a signal    execution/terminate (FR-017, FR-018)
              ▼                 ▼                  ▼
      Exited{code}      Signalled{signal}   Signalled{signal}
              └─────────────────┴──────────────────┘
                                │ ending reported AND retained output delivered
                                ▼                  (FR-022, FR-023)
                           ┌──────────┐
                           │ Released │  the id is free to reuse (FR-023)
                           └──────────┘
```

The attachment axis crosses every one of those states:

```
   Attached ──── connection drops (FR-031, A-TASKLIFE) ────▶ Detached
   Detached ──── execution/attach (FR-031b) ──────────────▶ Attached
```

A task may be `Running` and `Detached`, `Signalled` and `Detached`, or `Released` — at which point
attachment is meaningless because the task is gone. The one combination that cannot exist is
`Starting` and `Detached`: a task is created by a request, so a client is present at its birth by
construction.

**Whether more than one client may be attached at once is undetermined**, and spec.md says so
itself: "Two panels on one task. Whether a task has one viewer or many is a scope question." The
`attached: bool` on `Task` is the honest minimum that satisfies every stated requirement. A count
would be needed the moment the answer is "many"; nothing here forecloses that.

### Which transitions a client can see

This table is the part worth reading twice, because three of these happen where nobody is looking.

| Transition | Observable by a client | How |
|---|---|---|
| `Starting` → `Running` | Yes | The `runTask` response, carrying `pid` |
| `Starting` → spawn failure | Yes | The `runTask` **error**. Never an `onExit`, and SC-015 asserts zero (FR-004) |
| `Running` → ended, **attached** | Yes | `onExit`, after every chunk produced before it (FR-022, SC-011) |
| `Running` → ended, **detached** | **No** | Happens with nobody watching. Reported on the next `attach` (FR-031b, SC-020) — and §4.8's result has no field for it |
| Retention bound reached; reader stops | **No** | No notification exists. The process observes a blocking write; the client observes nothing (FR-013) |
| Bound relieved; reading resumes | **No** | As above |
| `Attached` → `Detached` | **No, not by the client** | The engine observes the connection dropping. A client does not observe its own disconnection at the moment it happens; it discovers it afterwards |
| `Detached` → `Attached` | Yes | The `attach` response, then the retained output, then whatever comes next (FR-031b, SC-019) |
| ended → `Released` | **No** | There is no notification. The client infers it from `onExit` plus delivery, and SC-014 measures it at the engine |

**The three unobservable transitions are not an oversight; they are what A-TASKLIFE bought.** A
task that keeps running while nobody is connected necessarily changes state where nobody can see
it, and the entire reattachment path — the retained bytes and the retained ending — exists to make
those changes recoverable after the fact rather than observable as they happen. FR-032 is the
requirement that the developer be *told* about them on reconnection rather than left to infer them
from a panel that resumed, which makes it a client-side obligation that this data model supplies
the inputs for: `running`, the retained bytes, and the ending that §4.8's row currently cannot
carry.

---

## What is NOT persisted

**Nothing. There is no new SQLite: not a table, not a column, not an index, not a migration.**
F004 took the client projection from v1 to v2; F010 leaves it at v2 and touches no schema. The
client's `PRAGMA user_version` is unchanged by this feature.

Everything in this document lives in **engine memory and dies with the engine process** — the
`TaskSet`, every `Task`, every `RetainedOutput`, every pseudo-terminal descriptor and the
`ResourceLimits` constant. That is the same lifetime the workspace registry has ("in memory, for
the engine's lifetime", `ports/roots.rs`) and the same lifetime F004's `WatchSet` has. Retained
output for a detached client is bounded memory, **not a file** (plan.md, Storage).

On the client side, a panel's scrollback is webview memory and goes when the panel goes. The only
durable client store is A-STATE's JSON file, and see below.

### What that means when the engine restarts — and what §15.2 actually says

§15.2 reads: "the engine tracks active task IDs, PID mappings and language server session state, so
a transient crash can be recovered rather than requiring the developer to rebuild their session by
hand." A-TASKLIFE cites that sentence in support of reattachment. Read against this plan, here is
what is true and what is not.

**True:** the engine does track active task ids and their pids, in the `TaskSet`, for as long as it
is running. That tracking survives a **disconnection**, which is what A-TASKLIFE needs and the only
thing it needs. The client reconnects, the engine still holds the set, and `attach` finds the task.

**Not true:** that a crash can be recovered. The mapping is in the process that crashed. There is
no file, no socket handed to anything, and no other holder of the id-to-pid relation. After a
crash the engine starts with an empty `TaskSet`; the processes it spawned are still running, still
in their own process groups, and **unreachable by `taskId` forever**. §15.2's sentence describes an
intention that nothing in F010's plan implements, and A-TASKLIFE inherits the overstatement by
citing it.

**The sharper case is not a crash at all — it is the engine's own update path.** §15.3 gives the
engine in-place binary replacement and re-execution, and `session.rs` carries the session identity
across it in `APEX_SESSION_ID` so a restart is distinguishable from a fresh session. A
re-execution replaces the process image: the `TaskSet` is gone, and the pseudo-terminal
descriptors are gone with it, since file descriptors Rust opens are close-on-exec. The child
processes are not gone. They are still running, still children of the same pid, and nothing can
now read their output, write their input or reap them.

That is FR-025 breached — "The system MUST NOT leave a process running that nothing is watching and
nothing can reach" — reached through the engine's own supported update, not through any failure.

**What the contract already provides for it**: `session/onRestart`'s `unpreserved` list, which
F002 built and which `SessionRegistry::new()` currently fills with an empty vector under the
comment "Nothing is supervised yet; F007 and F010 will have something to report here." F010's
entry is the task set: after a re-execution the engine must tell the client that its tasks did not
survive, because an empty `unpreserved` is a positive assertion that nothing was lost.

**What the contract does not provide**: any way to reach the surviving processes. There are two
honest resolutions and **spec.md, plan.md and research.md choose neither**:

- **Terminate every task before re-executing.** FR-025 holds and nothing is orphaned, at the cost
  of a toolchain update killing a twenty-minute build — which is the loss A-TASKLIFE was recorded
  to prevent, arriving by a different door.
- **Carry the tasks across the exec.** File descriptors survive `exec` when close-on-exec is
  cleared, and the id-to-pid map can travel in the environment exactly as `APEX_SESSION_ID`
  already does. This is what would make §15.2's sentence true, and it is a feature nobody has
  specified.

This is recorded here as undetermined rather than resolved, because choosing between them is a
decision about what a restart costs a developer, and Principle III puts that in Appendix A rather
than in a data model.

**What a client sees across any of this.** It reattaches with a remembered `taskId` and receives
`-32006`, which §4.4 defines as "Task not found **or** already exited". Those are the two cases it
most needs to tell apart: "your build finished while you were away" and "your build may still be
running on this host and I cannot reach it". One code cannot say which.

### What the client must remember, and where

FR-031d requires a client that has **restarted** — not merely reconnected — to be able to reach the
tasks it started. spec.md's Assumptions justify this on the grounds that "F002 already persists
session state across a client restart, so extending what it persists costs nothing and needs no
protocol."

Two corrections to that sentence, neither of which changes the conclusion:

- The durable client-side store is **A-STATE's** JSON file in the platform application-data
  directory, decided for F000 `app-shell`. F002 `daemon-bootstrap` owns session continuity across
  *engine* re-execution; the feature map says so explicitly, and adds "task recovery is F010".
- A-STATE enumerates its payload as "window geometry, region layout, open document references and
  focus". Task identities are not in it. Extending it is cheap, as the assumption says, but it is
  an edit to an F000 decision record that no artefact currently records.

The failure this leaves is bounded rather than permanent, and spec.md states the bound: a client
whose stored state is lost entirely cannot reach its running tasks, and A-EC2 stops the instance
after thirty minutes without interactive traffic, which reaps them. That bound is itself
conditional on F005 — A-TASKLIFE's second-order consequence is that the same idle stop kills a
*surviving* task thirty minutes after the disconnect it survived, unless F005 decides a running
task defers it.

---

## Quantities the plan owes

Three requirements say, in their own words, that a number must be **fixed in the plan**. plan.md
fixes none of them. Its only figures are 500 ms (quoted from SC-001 and SC-009), 1 MiB (quoted from
§4.1) and 50 MiB (quoted from SC-006) — every one of them carried from elsewhere, and none of them
a choice this plan made.

| Quantity | Required by | Stated in plan.md | Ceiling that constrains it |
|---|---|---|---|
| Output chunk **size** bound | FR-011; research.md, *Chunking* ("values fixed in the plan") | **No** | ≤ 512 KiB raw (A-BULKSIZE), because base64 expands by a third under §4.1's 1 MiB cap |
| Output chunk **time** bound | research.md, *Chunking* ("values fixed in the plan") | **No** | Must keep SC-001's 500 ms end to end, so materially below it |
| **Retention** bound before a process is slowed | FR-013a, FR-031a | **No** | Memory on the instance; SC-021 and SC-024 exercise it under 50 MiB |
| Panel **scrollback** bound | FR-029a | **No** | Client memory; SC-024 measures and prints it |
| Per-process **resource limits** | FR-006b | **No** — the mechanism is named, no value is | SC-026 requires a memory ceiling enforced within 2 seconds |

Two further plan-level decisions spec.md's Assumptions name, and plan.md does not take:

- **Which signal an interrupt sends, which a stop request sends, and what escalation follows a
  process that ignores one** (FR-015, FR-017; spec Edge Cases, "Terminating is a request until it
  is not"). `TerminateParams::signal` cannot be typed until this is answered.
- **Which shell the developer-facing terminal starts, and whether it is a login shell.**

These are listed rather than invented. FR-006b's own justification is the reason: "A limit nobody
chose is one nobody can defend when it fires." A data model that picked the numbers would be
choosing them in the wrong document, and FR-013a's phrasing — "not a judgement made per task" —
would be satisfied in letter by a judgement made per data model.

---

## Invariants

Things a test should be able to break and find something wrong.

| # | Invariant | Enforced by | Requirement |
|---|---|---|---|
| 1 | Starting under a live `TaskId` is refused and produces no second process | `TaskSet::start` rejects a present key | FR-031c, SC-022 |
| 2 | A command that cannot be started fails the request and emits no `onExit` | No `Task` is inserted until the spawn returns a pid | FR-004, SC-015 |
| 3 | Output arrives byte-for-byte, including invalid UTF-8 | `Vec<u8>` in the domain, base64 on the wire; no `String` on the path | FR-009, SC-003 |
| 4 | Output for one task arrives in the order produced | One reader thread per task; `FrameWriter` holds the lock for one frame | FR-010, SC-004 |
| 5 | No chunk produces a frame over §4.1's cap, including for output with no line break | The size bound, at or under A-BULKSIZE's raw ceiling | FR-011, SC-005 |
| 6 | With `pty: true`, `onStderr` carries zero bytes and `isatty` is true | One device; the runner gives the child one descriptor | FR-008, A-TASKSTREAM, SC-028 |
| 7 | With `pty: false`, the two streams are separable and `isatty` is false | Separate pipes | FR-008, FR-008a, SC-028 |
| 8 | Every chunk produced before an exit is delivered before the exit | `RetainedOutput` is drained before `ending` is emitted | FR-022, SC-011 |
| 9 | An ending is exactly one of `Exited` or `Signalled`, never both and never neither | The domain enum; the wire mapping reads `signal` first | FR-021, SC-010 |
| 10 | At the retention bound, zero bytes are dropped and the process is slowed instead | The reader stops reading; nothing else happens | FR-013, SC-021 |
| 11 | The bound and the behaviour are identical attached and detached | One `RetainedOutput`, with no branch on `attached` | FR-031a, SC-021 |
| 12 | A dropped connection terminates zero tasks | The `TaskSet` is not scoped to the transport and knows nothing of it | FR-031, SC-018 |
| 13 | A reattaching client receives every retained byte, in order, before anything produced since | FIFO drain on attach, ahead of live delivery | FR-031b, SC-019 |
| 14 | A task that ended while detached still reports how, to whoever reattaches | `RetainedOutput::ending` outlives the process | FR-031b, SC-020 |
| 15 | Terminating a task leaves zero of its processes running, at any depth | The signal goes to the process group, not the pid | FR-018, SC-012, SC-027 |
| 16 | Terminating an already-exited task succeeds | The set still holds it until release; a released id is a no-op, not an error | FR-019 |
| 17 | Closing a workspace leaves zero of its tasks running | `drain_for_workspace`, then terminate each | FR-024, SC-013 |
| 18 | After a hundred start-and-exit cycles the live-id count and the running-process count return to their starting values | `release` removes on delivery; the reaper leaves nothing | FR-023, SC-014 |
| 19 | A task's `env` appears in zero log lines and zero crash reports, including on a failed spawn | `Debug` elides it; the crash reporter redacts (A-OBS) | FR-005a, SC-025 |
| 20 | A `cwd` that escapes the workspace root is refused with `-32002`, independently of the client | `ResolvedPath`, which has no public constructor but `resolve` | FR-003, §4.7, Principle VI |
| 21 | One process exceeding its memory ceiling dies within 2 seconds and the engine survives | Per-process limits, soft **and** hard | FR-006, SC-026 |
| 22 | The pseudo-terminal is named in exactly one file | `engine/tests/pty_confinement.rs`, following `inotify_confinement.rs` | plan.md Structure Decision; research.md, *Confining the mechanism* |

Invariant 6 deserves the note F004 gave its second: it is asserted by running a real process that
calls `isatty` and by counting the bytes on the error stream, not by asserting that a flag was
passed. The failure it guards against — a pty that is created and then not given to the child on
both descriptors — is invisible to any test that only checks the parameter went through.

Invariant 12 is the one that would silently pass for the wrong reason if the `TaskSet` were ever
given a reference to the connection. It is asserted by dropping the transport and then reading the
set, never by asserting that no terminate was called.
