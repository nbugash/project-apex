# Feature Specification: Execution Terminals

**Feature Branch**: `feature/F010-execution-terminals`

**Created**: 2026-09-24

**Status**: Draft

**Input**: F010 `execution-terminals` from `specs/features-map.md` — starting a command with a
working directory and environment, as a real terminal rather than a pipe; streaming what it
writes as it writes it; typing back, resizing and stopping it; a panel that renders terminal
control sequences; and reporting how it ended, including when the connection goes away.

## On the source of values

Every number and rule below comes from somewhere. Where it comes from the system specification
it is cited; where this specification chooses it, that is said, and the reasoning is in
Assumptions.

- **§4.8** already defines all seven methods this feature needs — `execution/runTask`,
  `writeStdin`, `resizePty`, `terminate`, `onStdout`, `onStderr`, `onExit`. Unlike F004, which
  found `workspace/watch` absent, the catalogue is complete here and no amendment is expected.
- **§4.1** caps a frame at 1 MiB. A build's output exceeds that many times over, so output is
  chunked by definition.
- **§4.6** is the one that shapes this feature: one pipe is one queue, and outbound frames are
  priority-queued with interactive traffic ahead of background work. A terminal is the highest
  volume producer the channel will ever carry.
- **§7.3** sets the precedent for child processes: they are stopped cleanly on workspace close
  and on client disconnect, "so a dropped connection does not leave orphaned servers holding
  memory", and they run under resource limits that stop one runaway process destabilising the
  instance.
- **§8.3** — one panel per task, input flowing back and resize reaching the process, "so remote
  processes behave like a real terminal rather than a log viewer".
- **§15.2** — the engine tracks active task IDs and PID mappings so a transient crash can be
  recovered.
- **A-EC2** — single tenancy. A task runs as the developer's own user on their own instance.

## What this feature is not

- **Not general engine detachment.** F020 `detached-engine` owns whether the engine survives the
  client going away. This feature owns that a **task** does, and reattachment to it (A-TASKLIFE).
  That overlap is deliberate and was chosen over shipping a behaviour known to be wrong; it
  narrows F020 rather than removing it.
- **Not the language server supervisor.** §7.3 spawns children too, with its own lifecycle and
  resource policy. F007 owns that.
- **Not a task runner.** Nothing here discovers, configures or names tasks. It runs a command
  the developer or another feature supplies.
- **Not a shell.** A login shell is a command like any other; this feature does not implement
  one, configure one, or assume one.

## Clarifications

### Session 2026-09-24

- Q: A developer starts a 20-minute build, then their connection drops. What should happen to
  that running build? → A: It keeps running, and a reconnecting client reattaches and receives
  what it missed. §7.3's precedent — stop child processes on disconnect — is about language
  servers, which the developer never asked for and which restart invisibly. A build is twenty
  minutes of work they did ask for, and a dropped link is not a decision to abandon it.

  **Consequence, recorded rather than discovered during implementation**: this builds part of
  F020 `detached-engine` inside F010. That was the argument against it, and it was made
  deliberately — shipping a known-wrong behaviour to preserve a feature boundary is the worse
  trade. Recorded in the system specification as **A-TASKLIFE** rather than only here, because a
  decision that binds F020 is one F020 must be able to find. F020's remaining scope is the engine
  itself and everything that is not a task.

  **Second consequence, for F005**: A-EC2 stops the instance after thirty minutes without
  interactive traffic, and a detached task produces none — so without a change to idle detection
  the surviving task dies half an hour after the disconnect it survived. Whether a running task
  defers the idle stop is idle-detection policy, which F005 `ec2-lifecycle` owns. Recorded, not
  decided here: the trade is about billing a machine by the hour.

- **Coverage scan, no further questions.** A scan across functional scope, data model,
  interaction, non-functional attributes, integration, failure handling, trade-offs, terminology
  and completion signals found four gaps, each with a reasonable default, each recorded in
  Assumptions rather than asked: a task inherits the engine's environment with the caller's
  variables over it; the developer-facing entry point starts a shell; a client that restarts
  remembers what it started, with A-EC2's idle stop bounding the case where it does not; and
  concurrency is bounded by §7.3's existing resource limits rather than by a count.

  The one ambiguity with a genuine trade-off was task lifetime across a disconnection, and it was
  asked and answered above — front-loaded into the specify phase rather than left for this one,
  because it decided how much of another feature gets built inside this one.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Run a command and watch it work (Priority: P1)

A developer runs `cargo build` from the terminal panel. Output appears as it is produced —
progress bars redrawing, colours intact — not in a lump when the command finishes.

**Why this priority**: This is the feature. A remote development tool whose terminal shows
nothing until a build completes is a log viewer, and §8.3 rejects that explicitly.

**Independent Test**: Start a command producing steady output, and confirm the panel shows it
progressively, with no remote host and no network.

**Acceptance Scenarios**:

1. **Given** a workspace, **When** the developer runs a command, **Then** its output appears in
   the panel as it is produced rather than at completion.
2. **Given** a command writing to both output streams in a terminal, **When** it runs, **Then**
   both appear, merged as a shell merges them.
2a. **Given** the same command run without a terminal, **When** it runs, **Then** the two streams
   arrive distinguishable from one another.
3. **Given** a command emitting ANSI colour and cursor control, **When** it runs, **Then** the
   panel renders the result rather than the escape sequences.
4. **Given** a command that runs in a chosen directory, **When** it starts, **Then** it runs there
   and not in the workspace root by default.
5. **Given** a command needing an environment variable, **When** it is supplied, **Then** the
   process sees it.

---

### User Story 2 - Type back, and mean it (Priority: P1)

The developer answers a prompt, interrupts a runaway process with Ctrl-C, and resizes the panel
so a progress bar redraws at the new width.

**Why this priority**: P1 because it is what distinguishes a terminal from a transcript. §8.3
requires remote processes to "behave like a real terminal", which is a claim about input and
geometry as much as output.

**Independent Test**: Run a command that prompts, answer it, and confirm the process received
exactly what was typed; resize, and confirm the process observes the new dimensions.

**Acceptance Scenarios**:

1. **Given** a running process reading stdin, **When** the developer types, **Then** the process
   receives those bytes.
2. **Given** a running process, **When** the developer sends an interrupt, **Then** the process
   receives it as a signal rather than as literal text.
3. **Given** a running process, **When** the panel is resized, **Then** the process observes the
   new dimensions.
4. **Given** a process that checks whether it is attached to a terminal, **When** it runs here,
   **Then** it behaves as though it is.
5. **Given** a developer asking to stop a task, **When** they do, **Then** it is terminated and
   said to have been.

---

### User Story 3 - Know how it ended (Priority: P1)

The command finishes. The developer learns whether it succeeded, and the instance is not left
holding a process nobody is watching.

**Why this priority**: P1 because an exit nobody reports is indistinguishable from a hang, and a
process nobody reaps is a resource leak on a machine the developer pays for by the hour (A-EC2).

**Independent Test**: Run a command that exits non-zero, confirm the code is reported; run one
that is killed by a signal, confirm that is distinguishable from an exit code.

**Acceptance Scenarios**:

1. **Given** a command that completes, **When** it exits, **Then** its exit code is reported.
2. **Given** a command killed by a signal, **When** it dies, **Then** that is reported and is
   distinguishable from an ordinary exit.
3. **Given** a finished task, **When** it has exited, **Then** its resources are released and its
   identity is no longer live.
4. **Given** a workspace being closed, **When** it closes, **Then** its tasks are terminated
   rather than left running.

---

### User Story 4 - A build must not freeze the editor (Priority: P2)

A build emits tens of megabytes of output. The developer keeps typing, navigating and opening
files throughout, and none of it stalls.

**Why this priority**: P2 because it does not affect the common path, and P2 rather than P3
because the failure mode is the one the whole architecture exists to prevent. §4.6 states it
directly: one pipe is one queue, and a large payload serialises ahead of everything behind it.

**Independent Test**: Start a process emitting output far faster than a developer could read,
and confirm interactive actions continue to meet their budget throughout.

**Acceptance Scenarios**:

1. **Given** a task producing output continuously, **When** the developer performs an interactive
   action, **Then** it completes within the interaction budget.
2. **Given** a task producing more output than the link can carry, **When** that persists, **Then**
   the system has a stated behaviour rather than growing without bound.
3. **Given** output arriving faster than the panel can render, **When** it does, **Then** the panel
   stays responsive.

---

### User Story 5 - Come back to a build that kept going (Priority: P2)

A developer's connection drops twelve minutes into a build. They reconnect. The build is still
running, the panel fills in what they missed, and it carries on.

**Why this priority**: P2 because it does not affect the common path, and P2 rather than P3
because the alternative is losing work to a network blip — which is the failure A-TASKLIFE was
recorded to prevent, and the reason this feature reaches into F020's territory at all.

**Independent Test**: Start a long-running task, sever the connection, restore it, and confirm
the task never stopped and no output was lost.

**Acceptance Scenarios**:

1. **Given** a running task, **When** the connection drops, **Then** the task continues running.
2. **Given** a task that continued running, **When** the developer reconnects and reattaches,
   **Then** they receive the output produced while they were away, in order, before anything
   produced since.
3. **Given** a task that exited while nobody was attached, **When** the developer reattaches,
   **Then** they are told it ended and how.
4. **Given** an identity already running, **When** a client tries to start a task under it,
   **Then** it is refused rather than starting a second process.
5. **Given** a reconnection, **When** it completes, **Then** the developer is told which of their
   tasks survived and what they missed, rather than inferring it from a panel that resumed.

### Edge Cases

- **A command that does not exist.** The failure belongs to the request, not to a task that
  never started.
- **A command producing a single line megabytes long.** No newline to chunk on, and §4.1 caps a
  frame at 1 MiB.
- **A process that exits immediately.** The exit may be observed before the client has finished
  attaching a panel to it.
- **A process that ignores a request to stop.** Terminating is a request until it is not.
- **A process that spawns children.** Stopping the parent leaves the children, which is how an
  orphaned build keeps consuming the instance after the developer thinks it has stopped.
- **Output arriving after the process has exited.** Buffered bytes in flight when the exit is
  observed must not be lost or reordered past it.
- **Binary output.** A command that writes raw bytes rather than text, which must not be mangled
  into something that renders as replacement characters.
- **The connection dropping mid-task.** The task survives (A-TASKLIFE), which raises the rest of
  this list.
- **A task that finishes while no client is attached.** Its exit has nobody to report to at the
  moment it happens, and must still be reportable to whoever returns.
- **A client that never comes back.** The task runs until it ends on its own, or until A-EC2's
  idle stop takes the instance — which is thirty minutes after the disconnect unless F005 decides
  a running task defers it.
- **A client reattaching to a task that has already exited.** The output and the exit are still
  owed to it.
- **A client reattaching with an identity that was never started**, or starting one that is
  already running.
- **Output retained for an absent client reaching its bound.** The same slowing applies; the
  process must not behave differently for being unobserved.
- **The engine restarting under a running task.** §15.2 tracks task IDs and PID mappings for
  exactly this, and F002's session contract says what survives.
- **Two panels on one task.** Whether a task has one viewer or many is a scope question.
- **A task started in a directory that is deleted while it runs.**
- **An environment variable carrying a secret.** It is the developer's own instance and their own
  user (A-EC2), but it should not therefore appear in logs.

## Requirements *(mandatory)*

### Functional Requirements

**Starting a task**

- **FR-001**: A task MUST be startable with a command, a working directory and environment
  variables, and MUST be identified by an id the client chooses.
- **FR-002**: A process asking whether it is attached to a terminal MUST be told yes, and MUST
  behave accordingly — colour where it colours, progress bars where it draws them, prompting
  where it prompts. A pipe that merely carries bytes fails this, and §8.3 rejects that outcome
  in as many words.
- **FR-003**: A task MUST run within the workspace it names, and its working directory MUST be
  refused if it escapes the workspace root, checked by the engine independently of the client
  (Principle VI, §4.7).
- **FR-004**: Starting a task MUST report the failure of a command that cannot be started, and
  that failure MUST be distinguishable from a task that started and exited immediately.
- **FR-005**: A task MUST run as the developer's own user with no escalation (A-SEC, A-EC2).
- **FR-005a**: A task's environment MUST NOT be written to any log or crash report. It is a
  common place for credentials, and A-OBS already requires the crash reporter to redact — this
  states what it must redact here. Core dumps MUST be disabled for a task process: a dump is a
  crash report containing the entire environment, so leaving them enabled writes to disk exactly
  what this requirement forbids. The task's **command** is deliberately not covered; a credential
  passed as an argument is a real leak this requirement does not close, and redacting arguments
  would remove the one field that makes a failed task diagnosable. Recorded as a known boundary
  rather than left to be read as an oversight.
- **FR-006**: A task MUST be constrained by per-process resource limits, inherited by its
  children, so that a single runaway process cannot exhaust the instance and take the engine with
  it (A-LSP names that outcome as the one to prevent; A-TASKLIMIT records what this catches).
- **FR-006a**: A task MUST run in its own process group, so that stopping it stops everything it
  spawned (FR-018) rather than only the command that was named.
- **FR-006b**: The limits MUST be **stated quantities fixed in the plan**, not judgements made
  per task. A limit nobody chose is one nobody can defend when it fires.

**Output**

- **FR-007**: Output MUST be delivered as it is produced, not accumulated until the process
  exits.
- **FR-008**: Standard output and standard error MUST both be delivered. Whether they are
  **distinguishable** depends on the shape the caller chose: a task given a terminal has one
  device, so the two arrive merged, exactly as they do in any shell; a task given separate pipes
  has them separated, and its process is not attached to a terminal (A-TASKSTREAM).
- **FR-008a**: A caller MUST be able to choose which shape it gets, and the choice MUST be
  exclusive. The two cannot both hold for one task, and offering both would be offering something
  that does not exist.
- **FR-009**: Output MUST arrive byte-for-byte as the process wrote it, so ANSI escape sequences
  and non-text bytes survive the journey unmodified.
- **FR-010**: Output for one task MUST arrive in the order it was produced.
- **FR-011**: Output MUST be chunked so that no single delivery exceeds the frame limit (§4.1),
  including for output containing no line breaks.
- **FR-012**: Output delivery MUST NOT delay interactive traffic (§4.6, Principle V). A task
  producing output continuously MUST NOT prevent an interactive action from meeting its budget.
- **FR-013**: When a process produces output faster than it can be delivered, the system MUST
  slow the process rather than drop output or buffer without limit. A transcript with a hole in
  it is worse than a build that took longer, because the hole is silent and the delay is not.
- **FR-013a**: The amount buffered before a process is slowed MUST be a stated quantity fixed in
  the plan, not a judgement made per task. "Buffer a reasonable amount" is not implementable and
  not testable; the requirement is that memory held for one task is bounded by a number somebody
  chose.

**Input and control**

- **FR-014**: Bytes sent to a task's input MUST reach the process unmodified.
- **FR-015**: An interrupt MUST reach the process as a signal rather than as input text.
- **FR-016**: A resize MUST be observable by the process, so that output laid out to the terminal
  width is laid out to the new width.
- **FR-017**: A task MUST be terminable on request, and the request MUST state which signal it
  sends.
- **FR-018**: Terminating a task MUST also terminate its children, so that killing a build does
  not leave its compiler processes running.
- **FR-019**: A terminate request for a task that has already exited MUST succeed rather than
  fail, so that a client racing an exit is not told it did something wrong.

**Ending**

- **FR-020**: A task's exit MUST be reported, carrying its exit code.
- **FR-021**: A task killed by a signal MUST be reported as such, distinguishably from a task
  that exited with a code.
- **FR-022**: Output produced before an exit MUST be delivered before the exit is reported, so
  that the last lines of a failing build are not lost to the report of its failure.
- **FR-023**: A task's identity MUST be released once it has exited and its output has been
  delivered, and MUST NOT be reusable while still live.
- **FR-024**: Closing a workspace MUST terminate its tasks (§7.3).
- **FR-025**: The system MUST NOT leave a process running that nothing is watching and nothing
  can reach.

**The panel**

- **FR-026**: Each task MUST have its own panel instance (§8.3).
- **FR-027**: The panel MUST render ANSI colour, cursor movement and screen control rather than
  displaying the escape sequences.
- **FR-028**: The panel MUST remain responsive while output arrives faster than it can be read.
- **FR-029**: The panel MUST state when a task has ended and how, rather than simply stopping.
- **FR-029a**: The panel MUST retain a bounded history the developer can scroll back through, and
  the bound MUST be **a stated quantity fixed in the plan** rather than a judgement made per
  panel. "Keep a reasonable amount" is neither implementable nor testable, and an unbounded one
  makes a long build a memory leak on the developer's own machine.
- **FR-030**: The panel's appearance MUST come from the design system (Principle I), including
  the colours ANSI names — a terminal's palette is a design decision, not a default.

**Disconnection**

- **FR-031**: A running task MUST continue running when the client's connection drops
  (A-TASKLIFE). It MUST NOT be stopped merely because nobody is watching it.
- **FR-031a**: Output produced while no client is attached MUST be retained, bounded by the same
  limit that bounds output for an attached client (FR-013a). A task that outruns that bound while
  detached MUST be slowed, exactly as it would be while attached — the process must not discover
  it is unobserved by being treated differently.
- **FR-031b**: A reconnecting client MUST be able to reattach to a task by the identity it
  chose when starting it, and MUST receive the output produced while it was away, in order,
  before any output produced after.
- **FR-031c**: Reattaching MUST be distinguishable from starting, so that a client cannot
  silently start a second task under an identity that is already running.

  **This requires a way to attach to a task that is already running, which §4.8 does not
  define.** The catalogue has `execution/runTask`, which *starts* one, and six further methods
  that all address a task already known to the caller. There is nothing that says "this task
  exists; connect me to it". Recorded here as a consequence rather than discovered during
  implementation, the way F004 recorded `workspace/watch` — it is the sixth absence of this kind,
  and it exists because A-TASKLIFE made a task outlive the connection that started it.
- **FR-031d**: A client that has restarted, rather than merely reconnected, MUST be able to reach
  the tasks it started. What it remembers across its own restart is its own business; what it
  MUST NOT do is leave a task running with no way to reach it.
- **FR-032**: The developer MUST be told what happened to their tasks across a disconnection —
  which survived, and what was missed — rather than discovering it by inference from a panel that
  resumed or did not.

**Verification**

- **FR-033**: Every behaviour here MUST be verifiable with no remote host and no network,
  consistent with A-TEST.

### Key Entities

- **Task**: One running command. Has an identity chosen by the client, a command, a working
  directory, an environment, a process, and a lifetime that ends in an exit or a termination.
- **Terminal attachment**: What makes a task a terminal rather than a pipe, and what a process
  detects when it asks. Has dimensions that change, and carries the signals an interrupt turns
  into.
- **Output chunk**: A portion of what a task has written, with which stream it came from. Carries
  bytes, not text, and is ordered within its task.
- **Exit**: How a task ended — a code, or a signal. Distinct states, not one field with a
  convention.
- **Panel**: The developer's view of one task. Renders output, accepts input, reports dimensions.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Output appears in the panel within 500 ms of the process writing it, measured at
  the interface boundary.
- **SC-002**: A command emitting ANSI colour and cursor control produces a terminal cell grid —
  characters, foreground and background colour, and cursor position — matching a hand-written
  expected grid for each exercised sequence, in 100% of exercised cases. Stated as a grid rather
  than as "identical to a local terminal" because no local terminal exists to compare against:
  the repository has no such oracle, and comparing the panel against the same library rendering
  headlessly compares it with itself.
- **SC-003**: Bytes written by a process arrive byte-for-byte identical, including non-UTF-8
  sequences, with zero substitutions.
- **SC-004**: Output for one task arrives in the order produced, in 100% of exercised cases.
- **SC-005**: A single output line of 4 MiB is delivered completely, in chunks each within the
  frame limit, with zero truncation.
- **SC-006**: During a task emitting 50 MiB of output, interactive actions continue to meet
  §1.4's budget, measured and printed rather than asserted.
- **SC-007**: Typed input reaches the process byte-for-byte, with zero modification.
- **SC-008**: An interrupt reaches the process as a signal in 100% of exercised cases, and as
  literal text in zero.
- **SC-009**: A resize is observed by the process within 500 ms.
- **SC-010**: A task's exit code is reported in 100% of exercised cases, and a signal death is
  distinguishable from an exit in 100%.
- **SC-011**: Output produced before an exit is delivered before the exit report, with zero lines
  lost, across every exercised case.
- **SC-012**: Terminating a task leaves zero of its processes running, including children.
- **SC-013**: Closing a workspace leaves zero of its tasks running.
- **SC-014**: After a hundred start-and-exit cycles, the number of live task identities and of
  running processes returns to its starting value.
- **SC-015**: A command that cannot be started is reported as a start failure in 100% of
  exercised cases, and as a task exiting in zero.
- **SC-016**: The panel's colours resolve entirely to design-system tokens, with zero raw values.
- **SC-018**: A task running when the connection drops is still running afterwards, in 100% of
  exercised cases, with zero terminated by the disconnection alone.
- **SC-019**: A client reattaching receives every byte produced while it was away, in order and
  before anything produced after, with zero loss and zero reordering.
- **SC-020**: A task that exits while no client is attached has its exit reported to the client
  that reattaches, in 100% of exercised cases.
- **SC-021**: A detached task that outruns the retention bound is slowed rather than losing
  output, with zero bytes dropped — the same outcome as an attached one.
- **SC-022**: Starting a task under an identity already running is refused in 100% of exercised
  cases, and produces a second process in zero.
- **SC-023**: A client that restarts and reattaches reaches every task it started, in 100% of
  exercised cases, and leaves zero running tasks unreachable.
- **SC-024**: A panel's retained history stays within its stated bound across a task producing
  50 MiB, with the memory held measured and printed rather than asserted.
- **SC-025**: A task's environment appears in zero log lines and zero crash reports, including
  when the task fails to start.
- **SC-026**: A single process exceeding its memory limit is terminated within 2 seconds of
  doing so, and the engine survives in 100% of exercised cases.
- **SC-027**: Stopping a task leaves zero of the processes it spawned running, at any depth.
- **SC-028**: A task given a terminal reports `isatty` true and delivers zero bytes on the error
  stream, in 100% of exercised cases; the same task without one reports false and delivers its
  error output separately, in 100%.
- **SC-017**: The full suite for this feature runs with no remote host and no network.

## Assumptions

- **500 ms for output to appear (SC-001)** is this specification's choice, not a quoted
  requirement. §1.4 budgets interactions the developer *initiates*; watching output is not one.
  It is chosen as the threshold below which output reads as live rather than batched, and it
  leaves room above §18.1's modelled 250 ms round trip for the chunking this feature adds.
- **500 ms for a resize to be observed (SC-009)** follows the same reasoning: a redraw at the old
  width after a resize is a visible wrong, and half a second is the point at which it stops
  reading as the process's own lag.
- **4 MiB for the unbroken-line case (SC-005)** is four times §4.1's frame cap, chosen so the
  chunking is exercised across several frames rather than at exactly one boundary.
- **50 MiB for the interference case (SC-006)** is the order of magnitude a verbose build
  produces, large enough that any per-chunk cost compounds visibly.
- **Slowing the process rather than dropping output (FR-013)** is the behaviour a terminal
  already has: a program writing to a terminal nobody is reading from blocks when the buffer
  fills. Adopting it means a build under a slow link takes longer and its transcript stays
  complete, which is the trade a developer can reason about. The alternatives cannot be: dropping
  output produces a transcript that is wrong in a way nothing marks, and unbounded buffering
  moves a runaway build's memory cost onto the instance the developer pays for by the hour. The
  quantity buffered before slowing is a plan decision (FR-013a).

- **A hundred cycles for the leak case (SC-014)** because a leak of one identity or one process
  per cycle is invisible in a single pass and unmistakable in a hundred — the same reasoning
  F004's SC-009 used for watches.
- **A task inherits the engine's environment, with the caller's variables applied over it.**
  Starting from an empty environment would mean no `PATH`, and therefore no `cargo`, no `git` and
  no shell — every task would fail for the same reason. Inheriting is what a terminal does, and
  the override is what makes the environment parameter useful rather than decorative.

- **The developer can open a terminal and type into it**, and what it starts is a shell — a
  command like any other, in the terms of this specification. This is not a separate capability:
  it is the entry point without which the feature has no user, and it is the one F004 needs,
  because the change F004 exists to notice is a `git checkout` a developer types. Which shell,
  and whether it is a login shell, are plan decisions.

- **A client that restarts, rather than reconnecting, remembers the tasks it started.** The
  durable client store is A-STATE's, decided for F000 `app-shell`; F002 owns engine re-execution
  continuity, which is a different thing. A-STATE enumerates its payload as window geometry,
  region layout, open document references and focus — task identities are not among them, so this
  assumption requires extending an F000 decision record rather than relying on one. A client whose
  stored state is lost entirely recovers through `execution/list` (§4.8), which exists for exactly
  this case.

- **There is no fixed limit on concurrent tasks.** They are bounded by the per-process limits
  FR-006 applies, which is a real bound rather than an arbitrary count, and one a developer
  running two builds and a test watcher would not trip.

- **Per-process limits do not bound a process tree, and that is accepted** (A-TASKLIMIT). Sixty-
  four compilers at three gigabytes each exhausts a hundred-and-twenty-eight-gigabyte instance
  while every one of them stays under its own ceiling. On this hardware reaching that takes
  deliberate over-parallelisation — `-j$(nproc)` is sixteen — so the gap is recorded rather than
  closed, and closing it is the shared supervisor's job when one exists.

- The specific signals an interrupt and a stop request send, the escalation after a process
  ignores one, the mechanism that provides terminal attachment, the panel library, and the
  resource-limit mechanism are all **plan-level decisions**. They are named in the system
  specification and belong in `plan.md`, stated as values rather than judgements, in the same
  shape F004's coalescing window and bulk threshold took — and for the same reason F004's
  specification does not name the watch mechanism it uses.
