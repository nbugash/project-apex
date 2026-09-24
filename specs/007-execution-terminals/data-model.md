# Data Model: Execution Terminals

**Branch**: `feature/F010-execution-terminals` | **Date**: 2026-09-24 | **Plan**: [plan.md](./plan.md)

Two locations, one vocabulary, and **no third**. **Engine memory** holds every task, its bytes and
its ending, and dies with the process. **The wire** carries the nine `execution/*` messages §4.8
now defines, plus the one `workspace/*` row F010 adds because FR-024 needs a frame that means it.
There is no database, no file and no schema change: F004 took the client projection from v1 to v2,
and this feature takes it nowhere, because nothing here outlives the engine.

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
| **Unique across the engine, not within a workspace** | §4.8 |
| Unique among **live** tasks. Starting under an id that is already live is refused with `-32010`, and does not start a second process | FR-031c, SC-022 |
| Reusable once the task has been released — that is, once it has exited *and* its output has been delivered | FR-023 |
| Untrusted input, like every value off the wire. It is a map key and nothing else: it never reaches a path, a command line or an `exec` | Principle VI, §4.7 |
| Safe to log. The task's `env` is not (FR-005a), so the id is the correlation handle diagnostics use instead | FR-005a, SC-025, A-OBS |

**Engine-unique is now the catalogue's word, not this document's inference.** §4.8 states it
plainly — "A `taskId` is **unique across the engine**, not within a workspace" — and gives the same
reason this document had derived: six of the nine Execution rows address a bare `taskId`, so a
per-workspace identity would leave them unable to resolve a task at all. The `workspaceId` on
`runTask` and `attach` records **ownership, not namespace**, which is what FR-024 needs, since
closing a workspace must terminate that workspace's tasks and not another's. Two workspaces both
choosing `build` have named the same task, and the second `runTask` is refused rather than
starting a second process.

**What §4.8 still does not state, and this document does not invent**: any syntactic constraint on
the id. No length, no character set, no collision rule between two clients of one engine. Under
A-EC2 there is one developer per instance, so a collision is a client racing itself, which FR-031c
already refuses with `-32010`. A shared instance would make this a real question; A-WORKSPACE's
reversal condition is the same one.

### `Task`

One running command, and everything the engine must know to keep it running, reach it and end it.
FR-001, FR-002, FR-003, FR-005, FR-006, FR-006a.

| Field | Type | Rule |
|---|---|---|
| `id` | `TaskId` | Identity. Client-chosen; see above |
| `workspace` | `WorkspaceId` | Which workspace owns it. Set at `runTask` and never changed — it is what makes FR-024 a lookup rather than a search, and what `execution/list` reports per task |
| `command` | `Vec<String>` | Non-empty. `[0]` is the programme, the remainder are its arguments verbatim. **Argv, never a shell line** (§4.8) |
| `cwd` | `ResolvedPath` | Proven a descendant of the workspace's `CanonicalRoot` by the existing two-stage resolve. Defaults to the root when the caller names none |
| `env` | `BTreeMap<String, String>` | Overrides, applied **over** the environment the engine inherited (spec Assumptions). Ordered for reproducible spawning, not because order means anything |
| `pty` | `bool` | Fixed at start. Decides both `isatty` and the stream shape, and the two cannot be chosen separately (A-TASKSTREAM) |
| `pid` | `i32` | The child's process id, returned by `runTask`, `attach` and `list`. Known only once the spawn has succeeded |
| `pgid` | `i32` | The task's own process group. Equal to `pid` by construction, because the child is made leader of a new group (FR-006a) |
| `limits` | `ResourceLimits` | Applied to the child, inherited by its children (FR-006) |
| `state` | `TaskState` | `Running`, or a terminal `ExitStatus`. See *State transitions* |
| `retained` | `RetainedOutput` | The bounded buffer, and after the end, the terminal status awaiting delivery |
| `attached` | `bool` | Whether a client is currently receiving this task's output. Orthogonal to `state` |

**There is no window-size field, and that is deliberate.** `runTask` now carries `cols?` and
`rows?` (§4.8) and `resizePty` changes them afterwards, but the current size lives in the kernel's
`winsize` on the pseudo-terminal, which is the thing a process reads with `TIOCGWINSZ`. Copying it
into `Task` would create a second authority that can disagree with the first, and nothing in this
feature reads it back — `execution/list` does not report a size, and a reattaching client sends its
own panel's dimensions rather than asking what they were.

**`command` is argv, and §4.8 now says so.** The catalogue states it outright — "`command` is an
**argv vector**, not a shell line. The engine does not interpose `sh -c`" — and gives the reason
this document had derived from spec.md's *What this feature is not*: "**Not a shell.** A login
shell is a command like any other; this feature does not implement one, configure one, or assume
one." A single string would make quoting the engine's problem for input it is specifically
required not to interpret. The developer-facing terminal that spec.md's Assumptions describe is
then an ordinary task whose `argv[0]` names a shell — `$SHELL`, falling back to `/bin/sh`, and not
a login shell (plan.md, Fixed Quantities).

| Rule | Source |
|---|---|
| `cwd` off the wire is untrusted, resolved by the engine independently of the client, and refused with `-32002` if it escapes the root | FR-003, §4.7, Principle VI |
| Runs as the engine's own user, with no escalation and no set-uid path | FR-005, A-SEC, A-EC2 |
| `env` is never written to a log line or a crash report, including when the spawn fails | FR-005a, SC-025, A-OBS |
| Its own process group, so terminating it reaches everything it spawned | FR-006a, FR-018, SC-027 |
| A spawn that fails produces **no** `Task` at all; the `runTask` request fails with `-32011` instead | FR-004, SC-015 |

**Lifetime.** Created by `execution/runTask`, and only by a spawn that succeeded. Ends when the
process ends — by its own exit, by a signal, or by `execution/terminate`. Released, and its id
freed, once the end has been reported *and* the retained output delivered (FR-023). It survives
the client's connection dropping (FR-031, A-TASKLIFE) and does not survive the workspace closing
(FR-024), the engine exiting (FR-025), or the engine re-executing itself (A-TASKEXEC; see *What is
not persisted*).

**`env` is the one field with a handling rule attached to it.** FR-005a and SC-025 make it
unloggable, which means the `Task` cannot derive `Debug` naively: a `#[derive(Debug)]` on this
struct plus one `tracing` call carrying `?task` puts every variable in the log, and SC-025 asserts
zero. `Debug` must be written by hand to elide `env`, or `env` wrapped in a type whose `Debug`
elides itself. The second is harder to defeat by accident and is the shape this document
recommends; the choice belongs in design.md. FR-005a's core-dump clause is the same requirement
reaching the process rather than the logger — see `ResourceLimits`.

**`command` is outside that rule, by the requirement's own words.** FR-005a now states it: the
task's command "is deliberately not covered; a credential passed as an argument is a real leak
this requirement does not close, and redacting arguments would remove the one field that makes a
failed task diagnosable". This document previously flagged the asymmetry as an observation it
could not resolve; the specification has since resolved it as an accepted boundary. One
consequence is new and belongs here: `execution/list` returns `command` per task, so an argument
that carries a credential is now readable by any client that enumerates. Under A-SEC and A-EC2
that client is the same developer on a single-tenant instance, so this widens **where** the
accepted boundary is visible without widening **who** can see it. It would not survive a shared
instance, which is A-WORKSPACE's reversal condition again.

### `TaskSet`

The engine's live tasks. One per engine, **not one per workspace**, for the reason `TaskId` gives:
six of the nine Execution rows address a task by id alone.

| Field | Type | Rule |
|---|---|---|
| `tasks` | `BTreeMap<TaskId, Task>` | Ordered for determinism in tests and listings; nothing depends on the order, but `execution/list` returning a stable order costs nothing here and makes its tests writable |

| Operation | Effect | Behaviour on repeat |
|---|---|---|
| `start(task)` | Inserts a task the runner has already spawned | **Refuses** an id that is already present, as `-32010` (FR-031c) |
| `get(id)` | The task, live or ended-and-undelivered | — |
| `release(id)` | Removes a task that has ended and whose output is delivered | Releasing an absent id changes nothing and is not an error |
| `list(ws)` | Every task, or every task of one workspace, as a summary — the `execution/list` query | Read-only; repeating it changes nothing |
| `drain_for_workspace(ws)` | Every task of one workspace, for termination on close | Draining twice yields nothing the second time |
| `drain_all()` | Every task, for engine exit and for a re-execution (A-TASKEXEC) | As above |

**`list` and `drain_for_workspace` take the same argument and must not share an implementation.**
One answers a question and the other empties the set. They are adjacent enough that a helper
returning "the tasks of this workspace" is tempting; the risk is a listing that mutates, which is
invisible in a test that lists once. `list` takes `&self`.

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
| `list` returns exactly the ids present, so a client that lost its identities recovers all of them and no released one | FR-031d, SC-023 |
| After a hundred start-and-exit cycles the set returns to its starting size, and so does the count of running processes | SC-014 |
| Closing a workspace empties that workspace's share of the set and terminates each member | FR-024, SC-013 |
| No member survives the engine process, and none survives its re-execution | FR-025, A-TASKEXEC, plan.md Storage |

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
| Bounded at **64 KiB raw**, so no frame exceeds §4.1's cap, including for output containing no line break | FR-011, SC-005, plan.md Fixed Quantities |
| Emitted on the size bound **or** a **20 ms** time bound, whichever comes first | research.md, *Chunking, ordering, and what is pure*; plan.md Fixed Quantities |

**The size bound is 64 KiB raw and the plan now derives its ceiling rather than borrowing one.**
plan.md's reasoning is that §4.1 caps a frame at 1 MiB and base64 inflates by 4/3, so the true
ceiling is three quarters of the frame budget — about 786 KB — and a bound naively set to 1 MiB
overflows by a third. 64 KiB sits an order of magnitude under that ceiling, and under A-BULKSIZE's
512 KiB raw inline limit as well, while keeping a 50 MiB burst to roughly 800 frames rather than
the 6 400 an 8 KiB chunk would cost. The consequence for testing is arithmetic: SC-005's 4 MiB
unbroken line is **at least sixty-four chunks**, not the eight a 512 KiB bound would have made it,
so the test exercises the chunker across many boundaries rather than one.

**The time bound is 20 ms and exists for the case the size bound cannot serve.** A shell printing
a prompt and waiting never fills 64 KiB, so a size bound alone would never send it. 20 ms is below
the threshold at which a prompt reads as delayed and leaves almost all of SC-001's 500 ms to
transport and render; under a burst the size bound dominates and the time bound costs nothing.

**A chunk carries no sequence number, and ordering is structural rather than checkable.** One
reader thread owns one task's descriptor and one `FrameWriter` holds the lock for exactly one
frame, so chunks reach the wire in the order they were read and the transport preserves it. That
satisfies FR-010 and SC-004. The cost is worth naming: a client cannot *detect* a lost or
reordered chunk, because there is nothing to compare. This is acceptable for the same reason A-REQ
is — an in-flight request dies with its connection, and a connection that loses bytes without
dying is not a failure mode this transport has — but it does mean FR-031b's "in order, before
anything produced since" is a property of how the engine emits on reattachment, verifiable at the
engine and not at the client.

**Lifetime.** Between the reader thread's read and the frame writer's flush, plus however long it
waits in `RetainedOutput`. An `OutputChunk` is never persisted and never reaches a file.

### `ExitStatus`

How a task ended. **Distinct states, not one field with a convention** — that phrasing is spec.md's
own, in Key Entities, and FR-021 makes it testable. §4.8 now carries the same shape on the wire.

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
| Never constructed for a command that could not be started — that is a failed request carrying `-32011`, not an ending | FR-004, SC-015 |
| Reportable to a client that was absent when it happened, through `attach`'s result as well as through `onExit` | FR-031b, SC-020 |

**The wire now carries the distinction rather than encoding it.** `execution/onExit` is
`taskId`, `exitCode?`, `signal?`, and §4.8 states the invariant: **exactly one of the two is
present, never both, and never the `128 + n` convention.** The catalogue's reason is the one Key
Entities gave — a mandatory `exitCode` would leave a signalled death representable only through a
shell convention written for a human reading a number, not for a protocol a client must decode.
`execution/attach`'s result carries `exitCode?` and `signal?` under the same rule.

**The client's read rule, stated once and applied everywhere the pair appears**: *test `signal`
first; if it is present the task was signalled, whatever `exitCode` holds; if it is absent,
`exitCode` is present and the task exited.* Under §4.8's invariant the two orders agree, so this
rule costs nothing — and it is the order to write anyway, because it is the one that stays correct
against an engine that violates the invariant. A client testing `exitCode` first reads a
non-conforming both-present frame as an ordinary non-zero exit, which is precisely the confusion
FR-021 exists to prevent. It is the same conservative shape `RefusalReason` already uses.

**Neither present is a protocol violation and must be treated as one**, not as a third kind of
ending. There is no ending that is neither an exit nor a signal, so a frame carrying neither is a
frame this data model has no state for; the client surfaces it rather than inventing `Exited { 0 }`.

**Lifetime.** Created once when the process is reaped, held in `RetainedOutput` until delivered,
and gone when the task is released.

### `RetainedOutput`

What the engine is holding for a client that is not currently taking it. Bounded, in memory, and
the same structure whether a client is attached or not. FR-013, FR-013a, FR-031a, FR-031b.

| Field | Type | Rule |
|---|---|---|
| `chunks` | `VecDeque<OutputChunk>` | FIFO. Chunks rather than a flat byte buffer, so a `pty: false` task's two streams stay separable across a detachment |
| `bytes_held` | `usize` | Bounded at **4 MiB per task** (plan.md, Fixed Quantities). The bound is on **total bytes**, not on the number of chunks, because a chunk count bounds nothing when a chunk may be 64 KiB |
| `ending` | `Option<ExitStatus>` | Set when the process is reaped. This is what makes FR-023's "released once it has exited **and** its output has been delivered" implementable, and SC-020 satisfiable |

| Rule | Source |
|---|---|
| Bounded at 4 MiB per task — generous enough that a bursty producer is never slowed by a brief stall, bounded per task so concurrency cannot make it unbounded | FR-013a, plan.md Fixed Quantities |
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

**The 4 MiB bound and SC-005's 4 MiB unbroken line are the same number, and no test should rely on
that.** They were chosen independently — the retention bound as the memory one task may hold, the
test input as four times §4.1's frame cap — and they meet exactly. A single unbroken 4 MiB line
written to a task nobody is draining therefore reaches the retention bound on its last chunk. SC-005
asserts frame sizes and passes either way, but a test written to assume the whole line is buffered
before any assertion runs is a test that passes for the wrong reason.

**Replay on reattachment is the part where the catalogue's wording and SC-028 disagree**, and this
document takes the reading that keeps the requirement. §4.8 says the retained bytes "are replayed
as ordinary `onStdout` notifications after the response, in order". For a `pty: true` task that is
exactly right, because there is one device and `onStderr` is never emitted at all (A-TASKSTREAM,
SC-028). For a `pty: false` task it is not: the buffer holds `Stdout` and `Stderr` chunks
separately — that is why it is a queue of chunks rather than a flat byte buffer — and replaying
both on `onStdout` merges the two streams that FR-008a made exclusive and SC-028 measures. **The
rule this document states is that each retained chunk is replayed on the notification matching its
own `stream`**, which is what "ordinary notifications" ought to mean and what keeps live and
replayed output identical in shape as well as in order. §4.8's sentence should say so; the
discrepancy is recorded, not worked around.

**Lifetime.** With its `Task`. It is memory, never a file (plan.md, Storage), and it dies with the
process.

### `ResourceLimits`

The per-process ceilings a task runs under. FR-006, FR-006b, A-TASKLIMIT. plan.md now fixes every
member, so this is no longer a shape with one known field.

| Field | Type | Value | Why |
|---|---|---|---|
| `address_space` | `u64` | **16 GiB**, set as **both** the soft and the hard limit | Fatal to a runaway allocator under 128 GB, generous to any real build including a linker doing LTO. SC-026 is the one that measures it |
| `cpu_time` | — | **Not limited** | `RLIMIT_CPU` counts per process, so any value low enough to catch a spinning process kills a real compile. The runaway that takes the instance down is memory; a spinning process stays visible in `execution/list` and stoppable through `execution/terminate` |
| `core` | `u64` | **0** — core dumps disabled | **Required by FR-005a, not chosen.** A dump is a crash report containing the entire environment, so leaving dumps enabled writes to disk exactly what the requirement forbids |
| `nproc` | — | **Not limited** | `RLIMIT_NPROC` is per **user**, and under A-EC2 the engine runs as the same user as every task. Setting it for a task bounds the developer's whole session, the engine included |

| Rule | Source |
|---|---|
| Applied to the task's own process, between the fork and the exec | FR-006, A-TASKLIMIT |
| Inherited by everything the task spawns, at any depth | FR-006, A-TASKLIMIT |
| Stated quantities fixed in the plan, not judgements made per task — "a limit nobody chose is one nobody can defend when it fires" | FR-006b, plan.md Fixed Quantities |
| Identical for every task. Nothing in `runTask`'s parameters varies them, and §4.8 offers no field that could | §4.8, FR-006b |

**Inheritance is a property of the mechanism, not machinery this feature writes.** Resource limits
set on a process are carried across `fork` and preserved across `exec`, which is what makes
"inherited by its children" true without anything walking a tree. One consequence follows and is
load-bearing: a limit has a **soft** and a **hard** value, and a process may raise its own soft
limit up to the hard one. Setting only the soft limit therefore makes FR-006's constraint
advisory — a task that wants more memory can simply take it. **plan.md now states this and sets
both**, in as many words: "a process may raise its own soft limit up to the hard one, so setting
the soft limit alone makes the constraint advisory and a runaway task simply lifts it. This is
invisible until the first measurement of a runaway finds it was never bounded." This document
raised it as a gap; it is now a fixed quantity with the reasoning attached, and what remains here
is the invariant a test asserts (Invariant 21).

**Two of the four members are absences, and an absence is a decision that needs a test to stay
one.** "No CPU limit" and "no `RLIMIT_NPROC`" are as deliberate as the 16 GiB, and the failure mode
is somebody adding one later because it looked missing. The runner's limit set is a constant the
tests read, so adding a member is a visible change rather than a quiet one.

**What this does not bound, said plainly: a tree.** A-TASKLIMIT records it as accepted rather than
solved. Sixty-four compilers at three gigabytes each is a hundred and ninety-two gigabytes on a
hundred-and-twenty-eight-gigabyte instance, with **every single process under its own ceiling and
no limit exceeded**. The per-process limit catches the realistic runaway — one process with an
allocation bug — and cannot see the aggregate. Reaching the aggregate takes deliberate
over-parallelisation, since `-j$(nproc)` on this instance is sixteen, so the gap is recorded and
left to whichever feature builds the shared supervisor (§7.3, F007).

Two further things it does not bound, for completeness, because each is a question somebody will
ask of this type:

- **The number of concurrent tasks.** There is no count limit; spec.md's Assumptions say so and
  give the reason — the per-process ceilings are a real bound and a count is an arbitrary one.
- **Disk.** A task filling the workspace's filesystem is bounded by A-WORKSPACE's decision not to
  enforce a quota, and surfaces as an ordinary engine error.

**Lifetime.** A constant of the engine build, not per-task state. It is listed as an entity
because it is a value the `TaskRunner` port must be handed and a value tests must be able to
shrink; it holds nothing that changes.

---

## Wire types

For `protocol/src/wire.rs`, matching §4.8's Execution rows exactly as they now stand, plus
`workspace/close`, which is a Workspace row this feature adds because FR-024 needs a frame meaning
"I am finished with this workspace".

**On the naming convention, checked rather than assumed.** §4.8 states it: field names are written
camelCase in the tables and the wire carries snake_case. `wire.rs` implements that by doing nothing
special — no params or result struct carries `#[serde(rename_all)]`, the Rust field names are
already snake_case, and the only renames in the file are `kind` → `type` (a Rust keyword
collision) and the lowercase variants of `EntryKind`. F010's types follow by naming their fields
snake_case and adding no attribute. `taskId` is `task_id`, `exitCode` is `exit_code`.

### Output is bytes, and a JSON wire cannot carry bytes

This is the single most consequential shape decision in the feature, so it is stated before the
types rather than inside them. §4.8 now fixes it; what follows is why it is the only available
answer, and what it costs.

FR-009 and SC-003 require every byte a process wrote to arrive unmodified, "including non-UTF-8
sequences, with zero substitutions". JSON strings are Unicode. `serde_json` will not serialise a
Rust `String` containing invalid UTF-8 because a `String` cannot contain one, and the conversion
that would let it — `String::from_utf8_lossy` — substitutes U+FFFD, which SC-003 sets to zero.
**There is no text path.** `wire.rs` already reached this conclusion once, for
`ReadFileResult::content`: "There is no utf8 path: assuming text corrupts binary content silently,
and a method that sometimes returns text makes every caller branch on it."

So `data` on `writeStdin`, `onStdout` and `onStderr` is **base64 of the raw bytes**, carried in a
`String`, on all three payloads. §4.8 states it and states the absence this document previously
recorded as a fourth edit owed to the catalogue: "these payloads have no alternative encoding to
select between, so it is fixed here instead of offered."

**What that costs, precisely.** Base64 is four characters out for every three bytes in, a 33%
expansion before the JSON envelope. Three consequences follow:

1. **The chunk's raw size bound is derived from the expansion, not from the frame cap.** §4.1's
   1 MiB cap divided by 4/3 is roughly 786 KB, which is the true ceiling; A-BULKSIZE's 512 KiB raw
   inline limit sits under it. plan.md fixes **64 KiB**, an order of magnitude under both.
2. **Encode and decode are paid on the highest-volume traffic in the system.** §4.6 makes this one
   pipe and one queue, and F010 is, in plan.md's words, the largest producer the channel will ever
   carry. SC-006's 50 MiB becomes roughly 66.7 MiB of wire bytes, encoded once and decoded once.
   That cost is inside what FR-012 forbids delaying interactive traffic, and inside what SC-006
   measures.
3. **Nothing negotiates the encoding.** `workspace/readFile` carries an `encoding` field because a
   file may usefully be known to be text; process output never is, so these payloads carry no such
   field and the encoding is fixed.

### The types

```rust
// ---- execution/runTask ----      (request)
pub struct RunTaskParams {
    workspace_id: WorkspaceId,
    task_id:      TaskId,
    command:      Vec<String>,          // argv; [0] is the programme. Non-empty.
    cwd:          Option<String>,       // untrusted; the workspace root when absent
    env:          BTreeMap<String,String>, // serde(default); overrides over the inherited set
    pty:          bool,                 // A-TASKSTREAM: chooses isatty AND the stream shape
    cols:         Option<u16>,          // pty only; non-zero. Ignored when pty is false
    rows:         Option<u16>,          // pty only; non-zero. Ignored when pty is false
}
pub struct RunTaskResult { pid: i32 }

// ---- execution/attach ----       (request)
pub struct AttachParams { workspace_id: WorkspaceId, task_id: TaskId }
pub struct AttachResult {
    pid:       i32,
    running:   bool,
    retained:  u64,                     // BYTE COUNT, not bytes. Replayed after the response.
    exit_code: Option<i32>,             // exactly one of these two when running is false,
    signal:    Option<i32>,             // and neither when it is true
}

// ---- execution/list ----         (request)
pub struct ListParams { workspace_id: Option<WorkspaceId> }   // absent lists every task
pub struct ListResult { tasks: Vec<TaskSummary> }
pub struct TaskSummary {
    task_id:      TaskId,
    workspace_id: WorkspaceId,
    command:      Vec<String>,
    pty:          bool,
    pid:          i32,
    running:      bool,
    exit_code:    Option<i32>,          // same rule as AttachResult
    signal:       Option<i32>,
}

// ---- execution/writeStdin ----   (notification)
pub struct WriteStdinParams { task_id: TaskId, data: String }   // base64

// ---- execution/resizePty ----    (notification)
pub struct ResizePtyParams { task_id: TaskId, cols: u16, rows: u16 }

// ---- execution/terminate ----    (request)
pub struct TerminateParams { task_id: TaskId, signal: ??? }     // UNRESOLVED — see below
// result: empty

// ---- execution/onStdout, execution/onStderr ----  (notifications)
pub struct OutputParams { task_id: TaskId, data: String }       // base64

// ---- execution/onExit ----       (notification)
pub struct ExitParams {
    task_id:   TaskId,
    exit_code: Option<i32>,             // exactly one of the two is present,
    signal:    Option<i32>,             // never both and never neither (§4.8)
}

// ---- workspace/close ----        (request)
pub struct WorkspaceCloseParams { workspace_id: WorkspaceId }
// result: empty
```

**One params type serves both output notifications, and the stream lives in the method name.**
§4.8 gives `onStdout` and `onStderr` identical payloads — `taskId`, `data` — so a second struct
would be a duplicate whose only purpose is to be named differently. The domain's `OutputChunk`
carries a `stream` field because the domain needs to decide; the wire does not, because the method
has already said. That is also why replaying a `pty: false` task's retained output must choose the
method per chunk rather than send everything on one: the method *is* the stream field.

**`pty: true` means `onStderr` carries nothing, and the types must document that rather than
express it.** A-TASKSTREAM makes the choice exclusive: with a pseudo-terminal the task has **one
device**, `isatty` is true on both descriptors, and everything the process writes arrives merged on
`onStdout`. `onStderr` is not "usually empty" and not "rarely used" — for a `pty: true` task it is
never emitted at all, and SC-028 asserts zero bytes on it. With `pty: false` the two are separate
pipes, both notifications are used, and the process is not attached to a terminal. The type system
cannot carry this: `OutputParams` is one struct used by two methods, and no field of it varies
with `pty`. It is therefore a **doc comment on `RunTaskParams::pty` and on `OutputParams`**, and an
invariant a test asserts, which is the honest place for a constraint that spans two messages.

**`cols` and `rows` are optional on `runTask` and meaningful only when `pty` is true**, which §4.8
now states along with the reason: a process reads its terminal width at startup, before any client
has had an opportunity to resize it, so without them it reads whatever the pseudo-terminal
happened to be created with rather than a value somebody chose. They are `u16` and non-zero — the
width and height of a terminal are unsigned shorts in the structure the kernel takes, so a wider
type would only widen the range of values that must be rejected, and zero is meaningless where
some programmes divide by it. §4.8 states the optionality and the pty-only meaning; the type and
the non-zero constraint are derived here and belong in contracts/task-methods.md.

**`resizePty` against a `pty: false` task is silently ignored**, and §4.8 now says so: "there is no
terminal to resize, and a notification has no way to refuse". This document previously recorded the
behaviour as undefined with ignoring as the only thing a notification *can* do; the catalogue has
since chosen it. Nothing in `ResizePtyParams` changes — the rule is a doc comment and a test.

### `AttachResult::retained` is a byte count, and the replay is the delivery

§4.8 settles the question this document declined to answer, and settles it the way the constraint
pointed: `retained` is **a byte count, not the bytes themselves**. The catalogue gives three
reasons, each of which this document had recorded as a constraint — the retention bound (4 MiB) is
larger than §4.1's 1 MiB frame cap, chunking is defined for notifications rather than results, and
an exit delivered inside the result would arrive before the output that preceded it, which FR-022
forbids.

The retained bytes are replayed **after the response**, in order, as ordinary output notifications,
so one ordering rule covers live and replayed output alike (FR-031b, SC-019). Two properties follow
and both are testable:

- The count in the response is the number of bytes the client is about to receive, measured at the
  moment the replay begins. It is not a running total and not a high-water mark.
- The replay precedes anything the task produces after the attach, because the reader resumes
  behind the drain rather than beside it. This is the same FIFO the attached case uses.

**`AttachResult` now carries the ending, which closes the hole this document flagged.** FR-031b and
SC-020 require a task that exited while nobody was attached to have its exit reported to the client
that reattaches. The row was `{pid, running, retained}`, which carried `running: false` and no code
and no signal; it is now `{pid, running, retained, exitCode?, signal?}` under the same
exactly-one-of rule as `onExit`. §4.8 states the reasoning in the same terms: "`running: false` on
its own says only that it is over."

**`AttachParams::workspace_id` is now redundant, and a mismatch has no defined answer.** With
`taskId` engine-unique (§4.8), the engine resolves the task from the id alone; the `workspaceId`
confirms something it already knows. The catalogue keeps the parameter — it "records which
workspace owns the task" — but does not say what happens when a client attaches with the **wrong**
one. Three answers are available and they are not equivalent: `-32001` if the named workspace is
unregistered, `-32006` if the pair is treated as the key, or silence if the field is decorative.
This document does not choose; it is a contract question (contracts/task-methods.md) and it became
visible only once the id stopped being per-workspace. The safe reading, and the one Principle VI
points at, is that a mismatch is refused rather than ignored — an argument the engine accepts and
disregards is one a client can be wrong about forever.

### `execution/list`, and the record this document chooses

§4.8 adds the method, fixes its params (`workspaceId?`, omitted meaning every task the engine
holds) and names its result `{tasks[]}` **without defining the record**. The shape is chosen here
and justified, because there is nowhere else it is stated.

The method exists because `attach` takes an identity the caller must already know. A client that
has lost its identities — a fresh install, a cleared profile, a crash before its store was
written — has no route back to tasks that are still running, and under A-TASKLIFE those tasks keep
running. That is FR-025's abandoned process reached by a client doing nothing wrong, and SC-023
measures the recovery: "a client that restarts and reattaches reaches every task it started, and
leaves zero running tasks unreachable."

Each field earns its place against that job:

| Field | Why it is in the record |
|---|---|
| `taskId` | The whole point of the call. `attach` needs an identity; this is where a client that lost them gets them back (FR-031d, SC-023) |
| `workspaceId` | `workspaceId` is optional on the request, so an unfiltered listing must say which workspace each task belongs to or a client holding three of them cannot route the answer. The `Task` already holds it (FR-024) |
| `command` | The only human-readable thing about a task. FR-032 requires the developer be *told* what happened to their tasks; a list of opaque identities tells nobody anything, and a client cannot label a rebuilt panel without it |
| `pty` | Decides whether `onStderr` will ever carry bytes for this task (A-TASKSTREAM, SC-028) and whether `resizePty` does anything (§4.8: silently ignored otherwise). A client rebuilding a panel from a listing constructs it wrongly without this |
| `pid` | What `runTask` and `attach` both return, for the same diagnostic reason: it is the handle that survives outside the protocol |
| `running`, `exitCode?`, `signal?` | The same three fields `attach` returns, under the same rule. The set holds tasks that have ended and are awaiting delivery (FR-023), so a listing that said only "present" would conflate a running build with a finished one (SC-020) |

Three fields are deliberately **absent**:

- **`retained`.** A byte count changes with every read, so a value in a listing is stale before the
  client has read it. `attach` reports it at the moment the replay begins, which is the only moment
  it is true.
- **`attached`.** Whether more than one client may attach at once is undetermined — spec.md says so
  — and under one client the answer is "me" or "nobody", which the caller already knows.
- **`env` and `cwd`.** `env` is unloggable under FR-005a with SC-025 asserting zero, and a listing
  is the easiest way for it to reach a log at the other end. `cwd` is a path inside the workspace
  that nothing in FR-031d or SC-023 needs.

`TaskSummary` and `AttachResult` share five fields and are not one type: `attach` carries
`retained` and a summary carries identity, and merging them would put a count in a listing that
cannot be true or drop the count from the call that needs it.

### `workspace/close`

`workspaceId`, no result. It terminates that workspace's tasks and releases its watches (FR-024,
SC-013), and §4.8 is explicit that it is **not** the same event as a dropped connection: under
A-TASKLIFE a connection that drops leaves tasks running, because a laptop moving between networks
must not kill a build, whereas closing the workspace is the developer saying they are done with it.
"Conflating the two would make the protocol unable to express the difference between an accident
and an intention."

For this data model that sentence is the difference between two `TaskSet` operations: a dropped
connection sets `attached` false and nothing else; `workspace/close` calls
`drain_for_workspace(ws)` and terminates each member. The watch half belongs to F004's `WatchSet`
and is named here only because one frame drives both.

### `TerminateParams::signal` still has no stated encoding

plan.md now fixes the **signals**: an interrupt is `SIGINT`, a stop is `SIGTERM` escalating to
`SIGKILL` after 5 seconds, and both go to the process **group** rather than the process (Fixed
Quantities; FR-015, FR-017, FR-018, SC-027). That answers the question spec.md's Assumptions put in
the plan's hands.

What it does not answer is how the parameter is **written on the wire**. A name (`"SIGTERM"`) and a
number (`15`) are both defensible and they are not interchangeable across a protocol boundary; §4.8
names the parameter and not its type. This is the last undetermined field in the execution wire
surface, it is a contract decision rather than a quantity, and it belongs in
contracts/task-methods.md. This document records it rather than minting it.

### Error codes

`wire.rs`'s `codes` module holds six constants — `RESTARTING` (-32000), `WORKSPACE_NOT_REGISTERED`
(-32001), `PATH_REFUSED` (-32002), `NOT_FOUND` (-32003), `PAYLOAD_TOO_LARGE` (-32007) and
`WORKSPACE_GONE` (-32009) — and its own rule: "neither may write the integer inline — a literal
`-32001` in a match arm is a fact stated twice". F010 needs three that are now in §4.4 and absent
from the module.

| Code | Meaning | Where F010 returns it | Constant to add |
|---|---|---|---|
| `-32006` | **Task not found** | `attach` and `terminate` against an id the engine does not hold. **Not** `writeStdin` or `resizePty` — see below | `TASK_NOT_FOUND` |
| `-32010` | Task identity is already running — refused rather than starting a second process | `runTask` against a live id (FR-031c, SC-022) | `TASK_ALREADY_RUNNING` |
| `-32011` | Command could not be started — not found, not executable, or `cwd` unusable | `runTask` when the spawn fails (FR-004, SC-015) | `COMMAND_NOT_STARTED` |
| `-32001` | Workspace not registered | `runTask`, `attach`, `list` with a `workspaceId`, `workspace/close` | Already present |
| `-32002` | `cwd` escapes the workspace root | `runTask` (FR-003) | Already present |
| `-32009` | Workspace registered, root gone | `runTask` | Already present |

**`-32006` is now "Task not found" and nothing else**, and §4.4 explains what the narrowing fixed:
the old wording, "Task not found or already exited", contradicted two requirements at once.
Stopping a task that has already stopped is a success (FR-019), since the caller asked for it not
to be running and it is not running. And a client reattaching to a task that finished while it was
disconnected is entitled to learn how it finished (SC-020) rather than be told the task never
existed. An implementation following the old wording literally would have failed both and passed
review, because the wording was the specification.

**No code reaches `writeStdin` or `resizePty`, because a notification cannot carry one.** Both are
notifications (§4.8), so an unknown `taskId` is dropped silently, exactly as `resizePty` against a
`pty: false` task is. That is the only thing a notification can do, and it is worth stating because
it is the one place in this feature where a client can be wrong and never be told: a client writing
input to a task that has been released gets no error, no `onExit` and no acknowledgement. Its
recovery is `attach`, which is a request and does answer. Nothing requires a signal here and this
document does not invent one; it is recorded so that the silence is a known property rather than a
discovered one.

**`-32010` exists because `-32006` was its exact opposite.** FR-031c requires `runTask` under a
live identity to be refused, and a code meaning "not found" cannot carry "already there" without
making the two answers the same. `-32602` (invalid params) would technically carry it and would
make a client's recovery — attach instead of start — indistinguishable from a malformed request.

**`-32011` is not `-32003`.** §4.4 is explicit: `-32003` is reserved for paths inside a workspace,
and a programme name resolved against `PATH` is not a workspace path at all. A command that cannot
be started is the developer's mistake to fix, not the engine's failure, and SC-015 requires it to
be reported as a start failure in 100% of cases and as a task exiting in zero — which is only
testable if it has a code of its own.

**Neither the added methods nor the added codes increment `protocolVersion`.** A-PROTOVER and §4.8
are explicit that adding a method does not, and that both ends MUST ignore what they do not
recognise. What tells a client whether the engine has them is `auth/handshake`'s `capabilities`,
whose tokens are method names matched exactly — an engine without `execution/attach` in its set is
one a client must not assume it can reattach to, and one without `execution/list` is one a client
that lost its identities cannot recover against.

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
                           │ Starting │──── spawn fails (FR-004) ──▶ no Task; the REQUEST fails
                           └────┬─────┘                              with -32011 (never an onExit
                       spawned; │ pid and pgid known                  — SC-015)
                                ▼
                           ┌─────────┐◀──── writeStdin / resizePty ────┐
                           │ Running │                                 │
                           └────┬────┘──── 4 MiB retained ─────────────┘
                                │             (reader stops; the process blocks)
              ┌─────────────────┼──────────────────┬──────────────────────┐
     process exits     killed by a signal    execution/terminate    workspace/close,
              │                 │            (FR-017, FR-018)      engine exit, or a
              │                 │                  │               re-execution (A-TASKEXEC)
              ▼                 ▼                  ▼                      ▼
      Exited{code}      Signalled{signal}   Signalled{signal}      Signalled{signal}
              └─────────────────┴──────────────────┴──────────────────────┘
                                │ ending reported AND retained output delivered
                                ▼                  (FR-022, FR-023)
                           ┌──────────┐
                           │ Released │  the id is free to reuse (FR-023)
                           └──────────┘
```

**The rightmost branch reaches a terminal state but not always `Released`.** `workspace/close` ends
the task and the client is still connected, so the ending and the retained output are delivered and
the id is freed as usual (FR-023). An engine exit and a re-execution end the task with the same
signal, but there is no engine left to deliver anything — `Released` is skipped because the whole
set goes with the process. For a re-execution what the client learns instead is `unpreserved`
(A-TASKEXEC); for an exit it learns from the connection closing.

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
| `Starting` → spawn failure | Yes | The `runTask` **error**, `-32011`. Never an `onExit`, and SC-015 asserts zero (FR-004) |
| `Running` → ended, **attached** | Yes | `onExit`, after every chunk produced before it (FR-022, SC-011) |
| `Running` → ended, **detached** | **Not when it happens** | Reported on the next `attach`, in the result's `exitCode?`/`signal?` (FR-031b, SC-020), or in a listing's |
| Retention bound reached; reader stops | **No** | No notification exists. The process observes a blocking write; the client observes nothing (FR-013) |
| Bound relieved; reading resumes | **No** | As above |
| `Attached` → `Detached` | **No, not by the client** | The engine observes the connection dropping. A client does not observe its own disconnection at the moment it happens; it discovers it afterwards |
| `Detached` → `Attached` | Yes | The `attach` response, then the retained output replayed on the notification matching each chunk's stream, then whatever comes next (FR-031b, SC-019) |
| Terminated by a re-execution | Yes, afterwards | `session/onRestart`'s `unpreserved` names the task (A-TASKEXEC) |
| ended → `Released` | **No** | There is no notification. The client infers it from `onExit` plus delivery, and SC-014 measures it at the engine |
| Anything, to a client that lost its store | Yes, by enumeration | `execution/list` (FR-031d, SC-023) |

**The unobservable transitions are not an oversight; they are what A-TASKLIFE bought.** A task that
keeps running while nobody is connected necessarily changes state where nobody can see it, and the
entire reattachment path — the retained bytes and the retained ending — exists to make those
changes recoverable after the fact rather than observable as they happen. FR-032 is the requirement
that the developer be *told* about them on reconnection rather than left to infer them from a panel
that resumed, which makes it a client-side obligation that this data model supplies the inputs for:
`running`, the retained byte count, the ending, and — for a client with no identities left — the
listing that produces all three.

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

On the client side, a panel's scrollback is webview memory — **10 000 lines per terminal** (plan.md,
Fixed Quantities; FR-029a) — and goes when the panel goes. The only durable client store is
A-STATE's JSON file, and see below.

### What that means when the engine restarts — and what §15.2 now says

§15.2 has been narrowed to what the mechanism supports, and the narrowed version is what this data
model implements: **a disconnection is survivable and a crash is not.**

**Survivable, and this is the case A-TASKLIFE needs:** the engine tracks active task ids and their
pids, in the `TaskSet`, for as long as it is running. That tracking survives a **disconnection**.
The client reconnects, the engine still holds the set, and `attach` finds the task — or
`execution/list` finds it when the client no longer remembers what to ask for.

**Not survivable:** a crash. The mapping is in the process that crashed. There is no file, no
socket handed to anything, and no other holder of the id-to-pid relation. After a crash the engine
starts with an empty `TaskSet`; the processes it spawned are still running, still in their own
process groups, and reachable by pid and by nothing the protocol exposes. §15.2 says this in as
many words now, and A-TASKLIFE no longer cites the old crash-recovery sentence in support of
itself.

**`execution/list` does not close that case, and it is worth being exact about why.** It recovers a
client that lost its identities, not an engine that lost its task set: it can only enumerate what
the engine holds, and after a crash the engine holds nothing. SC-023's "leaves zero running tasks
unreachable" therefore holds for a restarted **client** against a surviving engine, which is the
case it describes, and not for a surviving client against a restarted engine. The residue is
FR-025's prohibition breached by a crash — recorded in §15.2 as the honest statement rather than
resolved, because resolving it needs the map to outlive the process and nothing in the system does
that today.

### The engine's own update path, which is now decided

§15.3 gives the engine in-place binary replacement and re-execution, and `session.rs` carries the
session identity across it in `APEX_SESSION_ID` so a restart is distinguishable from a fresh
session. A re-execution replaces the process image: the `TaskSet` is gone, and the pseudo-terminal
descriptors are gone with it, since file descriptors Rust opens are close-on-exec. The child
processes would not be gone.

This document previously recorded the resulting orphan as undetermined, between terminating the
tasks and carrying them across the `exec`. **It is now decided: A-TASKEXEC.** Before the engine
replaces its own binary and re-executes, it terminates every running task using the same escalation
`execution/terminate` uses — `SIGTERM` to the process group, `SIGKILL` after 5 s (plan.md, Fixed
Quantities) — and names each one in the `unpreserved` list of the `session/onRestart` notification
that follows. **Tasks do not survive a re-execution**, and §15.3 now says so.

What that means for this data model:

- `TaskSet::drain_all()` has a second caller. It is not only engine exit; it is the re-execution
  path, and the drained set is the input to `unpreserved`.
- `session.rs`'s `unpreserved: Vec<String>`, empty at every construction site under the comment
  naming F007 and F010 as its future callers, gets F010's entry: the task identities that were
  terminated. An empty `unpreserved` is a **positive assertion that nothing was lost** (§4.8), so
  leaving it empty after terminating a build is a lie the protocol has a field to avoid.
- FR-025 holds across the update path, because nothing is left running that nothing can reach.

The rejected alternative is recorded in A-TASKEXEC and is not reopened here: carrying the
descriptors across the `exec` by clearing `FD_CLOEXEC` and passing the map through the environment
would let a build survive an update, and was rejected because it makes every future change to the
task set a compatibility problem between two engine images, for a benefit available only during an
update the developer did not ask for and does not observe.

**What it costs the developer** is stated in the record rather than mitigated here: a twenty-minute
build running when the engine updates is lost. The mitigation is a client that declines to update
while tasks are running, which is client update-scheduling policy and belongs to whichever feature
owns it.

**What a client sees across any of this.** It reattaches with a remembered `taskId`. If the engine
survived, `attach` succeeds and reports the ending if there is one. If the task ran, ended and was
released, `attach` returns `-32006` — "Task not found", which after the narrowing means exactly
that and no longer doubles as "already exited", because a task that has exited and not yet been
delivered is still in the set and answers with its ending. If the engine was re-executed, the
client has already been told by `session/onRestart`. If the engine crashed, the client's resumption
is refused (§4.8: "A crashed engine sends nothing; its session is gone, and the client discovers
that when a resumption is refused"), which is how it learns not to trust the identities it holds.

### What the client must remember, and where

FR-031d requires a client that has **restarted** — not merely reconnected — to be able to reach the
tasks it started.

- The durable client-side store is **A-STATE's** JSON file in the platform application-data
  directory, decided for F000 `app-shell`. spec.md's Assumptions now attribute it there rather than
  to F002, which owns session continuity across *engine* re-execution — a different thing.
- A-STATE enumerates its payload as "window geometry, region layout, open document references and
  focus". Task identities are not in it. Extending it is cheap, as the assumption says, but it is
  an edit to an F000 decision record that this feature depends on.

**And the failure that remains is now bounded by a method rather than by a timer.** This document
previously recorded that a client whose stored state was lost entirely could not reach its running
tasks, and that A-EC2's thirty-minute idle stop would eventually reap them. spec.md now states the
recovery: "A client whose stored state is lost entirely recovers through `execution/list` (§4.8),
which exists for exactly this case." The idle stop is no longer the backstop for a lost client
store; it remains a live concern for a different reason, which A-TASKLIFE records as its
second-order consequence — the same idle stop kills a *surviving* task thirty minutes after the
disconnect it survived, unless F005 decides a running task defers it. That is an F005 decision and
is not this feature's to take.

---

## Quantities the plan fixes

Four requirements and one research decision say a value must be **a stated quantity fixed in the
plan** — FR-006b (resource limits), FR-013a (the amount buffered before a process is slowed),
FR-029a (the panel's retained history), FR-011 by way of research.md's *Chunking, ordering, and
what is pure* (the chunker's two bounds). plan.md's **Fixed Quantities** table now states all of
them, with the reasoning attached. They are reproduced here only where an entity above depends on
one; the table is the source and this document does not restate its justifications.

| Quantity | Value | Used by |
|---|---|---|
| Chunk size bound | 64 KiB raw | `OutputChunk` (FR-011, SC-005) |
| Chunk time bound | 20 ms | `OutputChunk` (SC-001) |
| Buffered before slowing | 4 MiB per task | `RetainedOutput` (FR-013a, FR-031a, SC-021) |
| Panel retained history | 10 000 lines per terminal | Client-side, outside this model (FR-029a, SC-024) |
| Task address space | 16 GiB, soft and hard | `ResourceLimits` (FR-006, SC-026) |
| Task CPU time | not limited | `ResourceLimits` (FR-006b) |
| Core dumps | disabled, `RLIMIT_CORE` = 0 | `ResourceLimits` (FR-005a) |
| Process count | not limited | `ResourceLimits` (FR-006b) |
| Interrupt signal | `SIGINT` | `execution/terminate` (FR-015) |
| Stop escalation | `SIGTERM`, then `SIGKILL` after 5 s, to the process **group** | `execution/terminate`, A-TASKEXEC (FR-017, FR-018, SC-027) |
| Developer shell | `$SHELL`, falling back to `/bin/sh`, not a login shell | `Task::command` (spec Assumptions) |

Two of these were raised by this document as gaps and are now closed by the plan rather than by
this file: the **soft-and-hard** rule on the address-space limit, which the plan states and
justifies in the same terms used above, and the **core-dump** setting, which FR-005a requires
rather than the plan choosing. One thing the plan fixes is a value and not an encoding: the signals
are named, and how `terminate` writes one on the wire remains the contract's to settle.

---

## Invariants

Things a test should be able to break and find something wrong.

| # | Invariant | Enforced by | Requirement |
|---|---|---|---|
| 1 | Starting under a live `TaskId` is refused with `-32010` and produces no second process | `TaskSet::start` rejects a present key | FR-031c, SC-022 |
| 2 | A command that cannot be started fails the request with `-32011` and emits no `onExit` | No `Task` is inserted until the spawn returns a pid | FR-004, SC-015 |
| 3 | Output arrives byte-for-byte, including invalid UTF-8 | `Vec<u8>` in the domain, base64 on the wire; no `String` on the path | FR-009, SC-003 |
| 4 | Output for one task arrives in the order produced | One reader thread per task; `FrameWriter` holds the lock for one frame | FR-010, SC-004 |
| 5 | No chunk produces a frame over §4.1's cap, including for output with no line break | The 64 KiB raw size bound, an order of magnitude under the 786 KB base64 ceiling | FR-011, SC-005 |
| 6 | With `pty: true`, `onStderr` carries zero bytes and `isatty` is true | One device; the runner gives the child one descriptor | FR-008, A-TASKSTREAM, SC-028 |
| 7 | With `pty: false`, the two streams are separable and `isatty` is false | Separate pipes | FR-008, FR-008a, SC-028 |
| 8 | Every chunk produced before an exit is delivered before the exit | `RetainedOutput` is drained before `ending` is emitted | FR-022, SC-011 |
| 9 | An ending carries exactly one of `exitCode` and `signal`, never both and never neither | The domain enum; the wire maps it, and the client reads `signal` first | FR-021, SC-010, §4.8 |
| 10 | At the 4 MiB retention bound, zero bytes are dropped and the process is slowed instead | The reader stops reading; nothing else happens | FR-013, SC-021 |
| 11 | The bound and the behaviour are identical attached and detached | One `RetainedOutput`, with no branch on `attached` | FR-031a, SC-021 |
| 12 | A dropped connection terminates zero tasks | The `TaskSet` is not scoped to the transport and knows nothing of it | FR-031, SC-018 |
| 13 | A reattaching client receives every retained byte, in order, before anything produced since, and `retained` equals the number it receives | FIFO drain on attach, ahead of live delivery; the count is taken at the drain | FR-031b, SC-019 |
| 14 | Replayed output arrives on the notification matching its own stream, so a `pty: false` task's stderr does not become stdout across a detachment | The replay reads each chunk's `OutputStream` | FR-008a, SC-019, SC-028 |
| 15 | A task that ended while detached still reports how, to whoever reattaches or lists | `RetainedOutput::ending` outlives the process; `attach` and `list` both carry it | FR-031b, SC-020 |
| 16 | Terminating a task leaves zero of its processes running, at any depth | The signal goes to the process group, not the pid | FR-018, SC-012, SC-027 |
| 17 | Terminating an already-exited task succeeds | The set still holds it until release; a released id is a no-op, not an error | FR-019 |
| 18 | `execution/list` returns every task the set holds and zero released ones, and an omitted `workspaceId` returns every workspace's | `TaskSet::list` reads the map it is | FR-031d, SC-023 |
| 19 | Closing a workspace leaves zero of its tasks running, and closes no other workspace's | `drain_for_workspace`, then terminate each | FR-024, SC-013 |
| 20 | An engine re-execution leaves zero tasks running and names every one of them in `unpreserved` | `drain_all` before the `exec`; the drained ids are the list | A-TASKEXEC, FR-025, §15.3 |
| 21 | After a hundred start-and-exit cycles the live-id count and the running-process count return to their starting values | `release` removes on delivery; the reaper leaves nothing | FR-023, SC-014 |
| 22 | A task's `env` appears in zero log lines and zero crash reports, including on a failed spawn | `Debug` elides it; the crash reporter redacts (A-OBS); `RLIMIT_CORE` = 0 leaves no dump to read it from | FR-005a, SC-025 |
| 23 | One process exceeding its 16 GiB ceiling dies within 2 seconds and the engine survives, and the ceiling cannot be raised from inside the task | Per-process limits, soft **and** hard | FR-006, SC-026 |
| 24 | A `cwd` that escapes the workspace root is refused with `-32002`, independently of the client | `ResolvedPath`, which has no public constructor but `resolve` | FR-003, §4.7, Principle VI |
| 25 | The pseudo-terminal is named in exactly one file | `engine/tests/pty_confinement.rs`, following `inotify_confinement.rs` | plan.md Structure Decision; research.md, *Confining the mechanism* |

Invariant 6 deserves the note F004 gave its second: it is asserted by running a real process that
calls `isatty` and by counting the bytes on the error stream, not by asserting that a flag was
passed. The failure it guards against — a pty that is created and then not given to the child on
both descriptors — is invisible to any test that only checks the parameter went through.

Invariant 12 is the one that would silently pass for the wrong reason if the `TaskSet` were ever
given a reference to the connection. It is asserted by dropping the transport and then reading the
set, never by asserting that no terminate was called.

Invariant 14 is the one §4.8's current wording would let fail. The catalogue says the retained
bytes are replayed "as ordinary `onStdout` notifications"; for a `pty: false` task that merges two
streams SC-028 requires separated. The test writes to both streams, detaches, reattaches, and
counts bytes per notification method — which is the only way to catch a replay that was written to
the sentence rather than to the requirement.
