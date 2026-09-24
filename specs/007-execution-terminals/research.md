# Research: Execution Terminals

**Branch**: `feature/F010-execution-terminals` | **Date**: 2026-09-24 | **Plan**: [plan.md](./plan.md)

Phase 0. Every unknown in the plan's Technical Context is resolved, and every decision closing a
genuine alternative names what it rejected and what would reverse it (Principle III).

Two decisions were made before this phase and are not repeated here: **A-TASKLIFE** (a task
outlives the connection that started it) and **A-TASKLIMIT** (bounded per process, not per tree).
Both are in Appendix A with their alternatives.

## Gate checks performed before research

**Principle IV — open items.** Run at the cycle's step 2 with the bounded detector: stop at
Appendix A, require a capitalised id. No live marker in any section F010 implements. The detector
was proven by injecting one and confirming it fired, because a detector that finds nothing and a
detector that is broken produce identical output. **Passes.**

**Principle II — contradiction.** Two found, both blocking, both resolved below.

---

## Attaching to a task that is already running

**Decision**: Add `execution/attach` to §4.8 — a request taking `workspaceId` and `taskId`,
returning the task's current state and the output retained since it was last read.

**As it landed**, §4.8's result is `{pid, running, retained, exitCode?, signal?}`, and two things
about it were settled after this decision was taken. `retained` is a **byte count**, not the bytes
themselves: the retention bound is larger than §4.1's frame cap and chunking is defined for
notifications rather than results, so the bytes are replayed after the response as ordinary
`onStdout` and `onStderr` notifications — each chunk on the notification its own stream would have
used when live — in order, and one ordering rule then covers live and replayed output alike. And
the result carries `exitCode?`/`signal?` under the same exactly-one-of rule as `execution/onExit`,
because a client reattaching to a task that finished while it was away learns how it finished from
the response, where `running: false` on its own says only that it is over. Neither changes the
decision below; both make "returning the output retained" a looser phrase than the wire allows,
and this paragraph is here so that phrase is not read as a specification.

**Rationale**: A-TASKLIFE made a task outlive its connection. The catalogue was written when it
did not: `execution/runTask` **starts** a task, and the other six methods address one the caller
already has. Nothing says "this exists, connect me to it". That is the sixth absence of this kind,
after `workspace/register` and `workspace/watch`, and it exists because a decision changed what
the protocol has to express.

Attaching must be a different call from starting, and that is the whole point rather than a
detail. FR-031c requires the two to be distinguishable: a client racing its own reconnection —
having reconnected, not yet certain what survived — must not silently start a second build under
an identity that already has one.

**Alternatives considered**:

- *Make `runTask` idempotent: the same id attaches rather than starts.* No new method, and the
  client needs no branch. Rejected because it makes the two outcomes indistinguishable at the
  call site, which is exactly the failure FR-031c names — a client that meant to start would be
  told it attached, and one that meant to attach would start.
- *`runTask` returns an error for a live id, and the client reads the error to decide.* Honest
  about the distinction and puts it in the failure path, where a client that forgets to handle it
  does the wrong thing quietly.

**Reversal conditions**: A task model where identities are engine-assigned rather than
client-chosen, which would make "already running" a question the client cannot ask.

---

## What the `pty` parameter means, and what it costs

**Decision**: `pty: true` gives the task a pseudo-terminal, and **its output arrives merged** on
`execution/onStdout`. `pty: false` gives it separate pipes, and standard output and standard
error arrive distinguishably on `onStdout` and `onStderr`. A caller chooses which it wants, and
the choice is exclusive.

**Rationale**: This resolves a conflict between two of this feature's own requirements, and the
conflict is a property of the mechanism rather than an oversight.

FR-002 requires a process asking whether it is attached to a terminal to be told yes. That means
its standard output and standard error are the same terminal device — which is what a terminal
*is*. FR-008 requires the two streams to be distinguishable. **Both cannot hold for one task.**
A real shell has exactly this property: run a build in a terminal and the two are interleaved
beyond separation, which is why `2>/dev/null` exists.

§4.8 anticipated this and the catalogue says so without saying so: `runTask` takes a `pty`
parameter, and defines both `onStdout` and `onStderr`. A flag that chooses between two output
shapes is the only reading under which all three of those facts are consistent.

The consequence for this feature: **the terminal panel uses `pty: true`** and everything arrives
on one stream, as it does in any terminal. A programmatic caller that wants to parse a build's
errors uses `pty: false` and gets them separated, at the cost of the process no longer believing
it has a terminal — which is the same trade every CI system makes.

**§4.8 must state this — and now does.** It named the parameter and never said what it did, the
same absence F004 found in the `event` vocabulary. The amended Execution subsection states both
shapes and that `onStderr` carries nothing when `pty` is true, and the choice is recorded as
**A-TASKSTREAM**. FR-008 is narrowed to match.

The same amendment gave `runTask` optional `cols` and `rows`, meaningful only when `pty` is true,
because a process reads its terminal width at startup — before any client has had the opportunity
to resize it — and made `execution/resizePty` against a `pty: false` task **silently ignored**,
a notification having no way to refuse. Neither follows from this decision; both are consequences
of it, and they are recorded here so a reader of this page is not surprised by the catalogue.

**Alternatives considered**:

- *Two pseudo-terminals, one per stream.* Technically possible and behaviourally wrong: processes
  expect their two streams to share a terminal, and `isatty` on both would be true while a resize
  applied to one.
- *A pty for output and a pipe for stderr.* The process sees a terminal on one descriptor and not
  the other, which is a state no real terminal produces and no program is written for.

**Reversal conditions**: None foreseen. This is how terminals work.

---

## The pseudo-terminal mechanism

**Decision**: `nix` in the engine, with the `term`, `process`, `resource` and `signal` features
only.

**Rationale**: A pseudo-terminal, a process group and per-process resource limits are four
syscall families, and the engine has no libc binding at all — F004 wrote `ENOSPC` as the literal
`28` rather than add one. Something has to provide them.

`nix` is a thin safe wrapper over exactly those calls, and taking four features rather than the
default set keeps what it compiles close to what it is used for. The engine runs only on Linux,
which F004's watcher already assumes, so the cross-platform alternative buys nothing here.

**Alternatives considered**:

- *`portable-pty`.* The system specification names it — in **§13.2, for local mode**, where tasks
  run in the client on the developer's own laptop across macOS, Linux and Windows. That is
  F015 `local-mode`'s dependency, in a different binary with a genuine cross-platform need, and
  it is the right choice there. In the engine it is weight for a portability that does not exist,
  against A-BOOT's concern about a binary transferred on every first connect.
- *Raw `libc` with hand-written `unsafe`.* Saves one crate and buys four families of unsafe code
  written once and reviewed rarely, to avoid a dependency that exists to get them right.

**Reversal conditions**: The engine gaining a non-Linux target, which makes the cross-platform
wrapper the cheaper option rather than the heavier one.

---

## A thread per task

**Decision**: One reader thread per task, owning its terminal descriptor, feeding the pure
chunker and writing through the `FrameWriter` F004 introduced.

**Rationale**: Reading a pseudo-terminal blocks, and the engine has no async runtime — a decision
its own manifest records, because it is transferred on every first connect. A thread per task
costs a stack when idle and nothing else, and there are as many tasks as a developer starts.

This is the same shape F004's watch thread took, and it reuses the same seam: `FrameWriter` is
already the one thing that speaks to the client, holding the writer for exactly one frame. F010
is the first feature to put real volume through it, which makes FR-012's measurement a
measurement of that seam under load rather than of this feature alone.

**Alternatives considered**:

- *One thread multiplexing every task with `poll`.* Fewer threads, and it is an event loop
  written by hand over a descriptor set that changes as tasks start and stop — the thing an async
  runtime exists to provide, without the testing an async runtime has had.

**Reversal conditions**: A task count high enough that per-thread stacks matter, which on a 128 GB
instance is not a number a developer reaches by working.

---

## Chunking, ordering, and what is pure

**Decision**: Output is chunked by **both** a size bound and a time bound: a chunk is emitted when
it reaches a stated size, or when a stated interval passes with bytes waiting, whichever comes
first. Both are values fixed in the plan, and plan.md's *Fixed Quantities* now fixes them:
**64 KiB** raw for the size bound, **20 ms** for the time bound. The chunker is pure — fed bytes
and told the time.

**Rationale**: Size alone starves an interactive process: a shell printing a prompt and waiting
writes far less than a chunk, and the developer would see nothing until they typed. Time alone
produces frames of unbounded size, which §4.1 caps at 1 MiB and FR-011 forbids. Each bound covers
the other's failure.

The size bound is stated in **raw** bytes rather than as a share of the frame cap because §4.8
makes `data` base64, which inflates by 4/3: the true raw ceiling is three-quarters of §4.1's
1 MiB, about 786 KB, and a bound naively set to 1 MiB overflows the frame by a third. 64 KiB sits
an order of magnitude under that ceiling, which is where plan.md put it and why.

Purity is what makes the volume requirements testable. Ordering, the size bound, the time bound
and the retention limit are all decidable without a process: the chunker is handed bytes and a
clock and asked what it emits — the same shape F004's coalescer took, and for the same reason.

**Alternatives considered**:

- *Line-based chunking.* Natural for text and wrong for a terminal: a progress bar redrawing with
  carriage returns emits no newline for minutes, and a single line can exceed the frame cap
  (SC-005 asserts 4 MiB explicitly).

**Reversal conditions**: Measured latency showing the time bound dominates, which tunes a number
rather than changing the shape.

---

## Backpressure comes for free

**Decision**: When retained output reaches its bound — **4 MiB** per task, fixed in plan.md's
*Fixed Quantities* — the reader **stops reading** the task's terminal. Nothing else is done.

**Rationale**: FR-013 requires a process outrunning the link to be slowed rather than truncated,
and this is the mechanism a terminal already has. A pseudo-terminal has a kernel buffer; when it
fills because nobody is reading, the process's next write blocks. The process is slowed by the
absence of a reader, which is precisely what happens to any program writing to a terminal nobody
is reading.

That it requires no mechanism is the point. Dropping output needs code to decide what to drop and
more code to say so; unbounded buffering needs none and moves a runaway build's memory onto the
instance. Not reading needs nothing and produces the behaviour the requirement asks for.

It applies equally while a client is detached (FR-031a). A process must not discover it is
unobserved by being treated differently.

**Alternatives considered**:

- *Drop the oldest retained output and mark the gap.* Keeps the process at full speed and makes
  the transcript wrong in a way only a marker records. A build log with a hole in the middle is
  worse than a build that took longer, because the hole is silent and the delay is not.

**Reversal conditions**: A task whose timing is the thing being measured — a benchmark — where
being slowed by its own observer invalidates the result. That is a different mode, not a
different default.

---

## Local mode is F015's

**Decision**: The client's local provider returns `Unsupported` for every task method. F010 ships
no local task execution.

**Rationale**: §13.2 describes local mode running tasks in a local pseudo-terminal against the
user's shell, and **F015 `local-mode` is the feature that owns it**, positioned after F010 in the
map. Building it here would put F015's core inside F010 — the trade A-TASKLIFE accepted for a
behaviour and A-TASKLIMIT rejected for a subsystem, and this is the second kind.

The precedent is exact: F004 reached the same conclusion for watching and recorded A-WATCHLOCAL.
The degradation is already specified rather than missing — the workspace stays usable and the
loss is stated.

**Alternatives considered**:

- *Implement local tasks now, since the client will need the machinery anyway.* It is a second
  pseudo-terminal implementation on three platforms, serving a mode with a named owner and no
  requirement in this feature's scope.

**Reversal conditions**: F015 arriving and finding the port the wrong shape, which is a change to
an adapter rather than to anything above it.

---

## Confining the mechanism

**Decision**: The pseudo-terminal is named in exactly one file,
`engine/src/adapters/outbound/pty_runner.rs`, enforced by `engine/tests/pty_confinement.rs`.

**Rationale**: plan.md states the rule, and prose does not fail a build. F004 established the
pattern and it justified itself immediately: the guard caught its own author three times — twice
on comments explaining why the library must not be named elsewhere, and once on the composition
root, which was resolved by moving the factory into the adapter rather than adding a judgement
call to the rule.

The rule is what makes the volume requirements testable at all. If the mechanism may be named
anywhere, then anywhere may decide something, and deciding requires a real process to test.

**Reversal conditions**: None. The guard is cheap and it has already paid for itself once.

---

## Appendix A records required before implementation

Two were already recorded when this phase ran — **A-TASKLIFE** and **A-TASKLIMIT**. One more was
owed, and it has since been written:

| Record | Decision | Reversal condition | Status |
|---|---|---|---|
| **A-TASKSTREAM** | `pty: true` merges the streams and gives the process a terminal; `pty: false` separates them and does not. The choice is exclusive because a terminal is one device | None foreseen; this is how terminals work | **Recorded** |

A second record was written afterwards, by the reconciliation that applied the edits below, and it
is named here because it changes what §15.3 promises a running task rather than anything this
phase decided: **A-TASKEXEC** — an engine re-execution terminates every running task and names it
in `session/onRestart`'s `unpreserved` list, rejecting the alternative of carrying the
pseudo-terminal descriptors across the `exec` by clearing `FD_CLOEXEC`. Appendix A now holds 39
records.

The system specification needed three edits. Five landed:

| Section | Edit | Why | Status |
|---|---|---|---|
| §4.8 | `execution/attach` row | A-TASKLIFE made a task outlive its connection; nothing reaches one the client did not start this session | **Landed** |
| §4.8 | State what `pty` does, and that `onStderr` carries nothing when it is true | The catalogue names the parameter and never defines it — the same absence F004 found in the `event` vocabulary | **Landed**, recorded as A-TASKSTREAM |
| §13.2 | Note that local task execution is F015's, not F010's | So a reader of §13.2 does not believe it ships with this feature | **Landed** |
| §4.8 | `execution/list` row | `attach` takes an identity the caller must already know, so a client that lost its store had no route back to a task still running — the seventh absence of this kind | **Landed**; not asked for by this phase |
| §4.8 | `workspace/close` row | FR-024 requires closing a workspace to terminate its tasks, and no frame meant "I am finished with this workspace" | **Landed**; not asked for by this phase |

Four clarifications landed in §4.8 alongside them, and they bear on the decisions above rather
than merely accompanying them: `data` is **base64** on `writeStdin`, `onStdout` and `onStderr`;
`command` is an **argv vector** rather than a shell line, so the engine interposes no `sh -c`; a
`taskId` is unique **across the engine** rather than within a workspace; and a `signal` on the
wire is the signal's **name** rather than its number, with `SIGTERM` escalating to `SIGKILL` and
`SIGINT` deliberately not escalating. §4.4 gained `-32010` (the identity is already running) and
`-32011` (the command could not be started), and narrowed `-32006` to "task not found" alone —
which is what makes FR-031c's refusal distinguishable in practice rather than only in principle.
