# Implementation Plan: Execution Terminals

**Branch**: `feature/F010-execution-terminals` | **Date**: 2026-09-24 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/007-execution-terminals/spec.md`

## Summary

A developer can run a command on the host and watch it work — output as it is produced, input
they type reaching the process, a resize the process observes, and an exit they are told about.

Three parts. The engine spawns each task under a pseudo-terminal in its own process group, with
per-process resource limits, and owns it for the task's lifetime rather than the connection's
(A-TASKLIFE). A reader thread per task turns raw bytes into chunked notifications through the
frame writer F004 built, which is already the one thing that speaks to the client. The webview
renders each task in its own panel, themed from the prototype rather than from the terminal
library's defaults.

Unlike F004, §4.8 already defined most of the methods. Three were missing and this feature adds
them: nothing in the catalogue attached to a task already running, which A-TASKLIFE made necessary
the moment a task outlived its connection; nothing enumerated tasks, which a client that has lost
its identities needs to reach them at all; and nothing said a workspace had been closed, which
FR-024 requires in order to stop that workspace's tasks.

## Technical Context

**Language/Version**: Rust with MSRV **1.75** across `protocol`, `engine` and `client/core`;
TypeScript 5.x with Svelte 5 in the webview.

**Primary Dependencies**: `nix` in the engine, with narrow features — `term` for the
pseudo-terminal, `process` for the process group, `resource` for the limits, `signal` for
delivering them. Chosen over `portable-pty`, which is cross-platform and therefore weight for
nothing: the engine runs only on Linux, F004's watcher already assumes it, and A-BOOT makes
binary size a first-class concern on something transferred on every first connect. **No async
runtime is added**, for the same reason F004 added none.

Webview: `@xterm/xterm` 6 with `@xterm/addon-fit`. §8.3 names the library; the prototype names
the appearance, and those are different things — see Principle I below.

**Storage**: None new. A task's state is engine memory for its lifetime, like the workspace
registry and F004's watch set. Retained output for a detached client is bounded memory, not a
file.

**Testing**: `cargo test --workspace` at Principle VII's three levels — unit against in-memory
fakes, integration across the protocol boundary, and the mock SSH daemon from F001, whose
`notify` directive F004 added and which this feature is the second user of. `npm run test:unit`
for TypeScript, WebdriverIO end-to-end under `tests/e2e/`. Every level runs with no remote host
and no network (A-TEST, FR-033).

**Target Platform**: Engine on Linux; client on macOS, Linux and Windows. The pseudo-terminal is
Linux-only by construction and sits behind a port, so nothing outside its adapter knows.

**Performance Goals**: Output reaches the panel within **500 ms** of the process writing it
(SC-001, a value this specification chose — see its Assumptions). A resize is observed within
500 ms (SC-009). Interactive actions keep §1.4's budget through a task emitting 50 MiB (SC-006).

**Constraints**: One pipe is one queue (§4.6), and this feature is the highest-volume producer
the channel will ever carry — the reason FR-012 exists and the thing Principle V measures. No
frame exceeds 1 MiB (§4.1), so output is chunked by definition. A process that outruns the link
is slowed, never truncated (FR-013). Per-process limits bound a single runaway; a tree that
collectively exhausts is recorded as owed (A-TASKLIMIT). **The engine cannot receive a
notification today**: `rpc.rs`'s `dispatch` reads `id` and returns `Action::Nothing` before the
method match, so an id-less frame is dropped silently. `execution/writeStdin` and
`execution/resizePty` are the catalogue's first client-to-engine notifications, so an id-less
dispatch path is foundational work for this feature rather than part of any one story. (The same
line reads `id` with `as_str()`, so a numeric id — legal under JSON-RPC 2.0 — is also dropped as
though it were a notification. Noted, not fixed here: no client sends one.)

**Fixed Quantities**: Four requirements say a value must be "a stated quantity fixed in the plan"
— FR-006b (resource limits), FR-013a (the amount buffered before a process is slowed), FR-029a
(the panel's retained history) and **FR-011**, which binds the chunker's two bounds by way of
research.md's *Chunking, ordering, and what is pure*, as data-model.md's *Quantities the plan
fixes* states. The fourth used to go unnamed here, which left this preamble listing three and then
counting five. They are fixed below, with the reasoning, because a number chosen during
implementation is a number nobody reviewed. The table carries more than those five: a quantity
found to be missing while the contracts were written is fixed here too, on the same grounds.

| Quantity | Value | Why this value |
|---|---|---|
| Chunk size bound | **64 KiB** raw | §4.1 caps a frame at 1 MiB and base64 inflates by 4/3, so the true ceiling is 3/4 of the frame budget — about 786 KB, and a bound naively set to 1 MiB overflows by a third. 64 KiB sits an order of magnitude under that ceiling while keeping a 50 MiB burst to roughly 800 frames rather than the 6 400 an 8 KiB chunk would cost. |
| Chunk time bound | **20 ms** | The size bound alone starves an interactive prompt, which never fills a chunk and would therefore never be sent. 20 ms is below the threshold at which a prompt reads as delayed, is negligible against the round-trip to the instance, and leaves almost all of SC-001's 500 ms to transport and render. Under a burst the size bound dominates, so this costs nothing where volume is high. |
| Buffered before slowing | **4 MiB** per task | research.md makes backpressure the absence of a mechanism: the engine stops reading, the pseudo-terminal's buffer fills, and the process blocks in `write` as it would against any slow consumer. This is how much the engine holds before that happens. Generous enough that a bursty producer is never slowed by a brief stall, bounded per task so concurrency cannot make it unbounded. |
| Panel retained history | **10 000 lines** per terminal | The library's default of 1 000 is too few to scroll back through a compile, which is the thing a developer most often wants to re-read. At roughly 200 bytes a line this is about 2 MB per terminal, which is what SC-024 measures against. |
| Task address space | **16 GiB**, soft **and** hard | Under 128 GB this is fatal to a runaway allocator and generous to any real build, including a linker doing LTO on a large workspace. Both limits are set deliberately: a process may raise its own soft limit up to the hard one, so setting the soft limit alone makes the constraint advisory and a runaway task simply lifts it. This is invisible until the first measurement of a runaway finds it was never bounded. |
| Task CPU time | **Not limited** | Deliberate. A legitimate build burns CPU for minutes and `RLIMIT_CPU` counts per process, so any value low enough to catch a spinning process is low enough to kill a real compile. The runaway that actually takes the instance down is memory; a process spinning on CPU stays visible in the task list and stoppable through `execution/terminate`. |
| Core dumps | **Disabled** (`RLIMIT_CORE` = 0) | Not a tuning choice. FR-005a forbids a task's environment reaching any log or crash report, and a core dump is a crash report containing the whole environment. Leaving dumps enabled writes the thing FR-005a prohibits straight to disk. |
| File size | **Not limited** | Deliberate, and recorded because every other resource here is either bounded or explicitly declined and disk was neither. `RLIMIT_FSIZE` caps a **single file**, and a build legitimately writes large ones — a linked binary, a container layer, a test corpus — so any value low enough to stop a runaway log is low enough to break real output. A task can therefore fill the volume. §5.5 and §16 already accept an unquota'd disk under single tenancy, which covers the consequence; this row records that the limit was declined rather than forgotten. |
| Process count | **Not limited** | `RLIMIT_NPROC` is per **user**, not per process, and under A-EC2 the engine runs as the same user as every task it starts. Setting it for a task bounds the developer's entire session, the engine included. A limit that can starve the engine is not a limit that protects it. |
| Default terminal size | **80 × 24** | `cols`/`rows` are optional on `runTask` and the kernel's default for a new pseudo-terminal is 0 × 0 — a size no display has, and the one value `resizePty` refuses, so the unstated case would otherwise land on the single illegal value. 80 × 24 is the conventional default every terminal program already expects to cope with. |
| Interrupt signal | `SIGINT` | What Ctrl-C sends. FR-015's interrupt is the developer asking the foreground process to stop, which is the signal every interactive program already handles. |
| Stop escalation | `SIGTERM`, then `SIGKILL` after **5 s** | Sent to the process **group**, not the process, so a shell's children go with it — the process group is the whole reason FR-018 can be met without cgroups. Five seconds is long enough for a build to flush and remove partial output, short enough that a developer who asked twice is not left waiting. |
| Developer shell | `$SHELL`, falling back to `/bin/sh` | The developer's own shell is what makes the terminal theirs; `/bin/sh` is guaranteed to exist when the variable is unset. Not a login shell: a login shell re-reads profile scripts whose side effects the developer did not ask for on every new terminal. |

**Scale/Scope**: A build emitting tens of megabytes over minutes, on a 16 vCPU / 128 GB instance
(§1). Concurrency is whatever the developer starts — two builds and a watcher is ordinary —
bounded by resource limits rather than a count.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Applies | How this feature satisfies it |
|---|---|---|
| **I. Design Fidelity** | **Yes — and the prototype is more specific than expected** | The prototype has a terminal dock, and it is the default one (`dock:'terminal'`). It specifies JetBrains Mono at 12.5px, line-height 1.6, padding `2px 12px 12px`, an accent-coloured shell prompt, and a 7×15px block cursor blinking on `vkpulse 1.1s steps(1,end) infinite`. Its title even switches on mode — `Terminal — build-01.euw1` against `Terminal — local`. **12.5px is a third font size**, distinct from `--vk-fs` and `--vk-code` at 13.5px, so `ds-sync` must extract it rather than a component inventing it. §8.3 names the library and Principle I names the appearance; the library is themed to the prototype, never the reverse, and its default palette is a violation like any other raw value. |
| **II. One Source of Truth** | **Yes — amendments landed** | §4.8 defined seven `execution/*` methods and **none attached to a task already running**; A-TASKLIFE made a task outlive its connection, so reattachment was required (FR-031b) and undefined — the sixth absence of this kind, after `workspace/register` and `workspace/watch`. `execution/attach` was added in Phase 0. Phase 1 found two more: `execution/list`, without which a client that has lost its identities cannot reach running tasks (SC-023), and `workspace/close`, which FR-024 and SC-013 both name. All three have landed, along with the encoding, arity and exit-shape statements the rows needed to be implementable. |
| **III. Decisions Recorded** | **Already satisfied, and re-checked here** | Two decisions closing genuine alternatives are recorded: **A-TASKLIFE** (a task outlives its connection) and **A-TASKLIMIT** (bounded per process, not per tree). Both were made before this plan, both carry rejected alternatives and reversal conditions. A third is owed and named in Phase 0: the attach method's shape. |
| **IV. Open Items Block** | Passes | Checked at the cycle's step 2 with the bounded detector — no live `[OPEN: id]` in any section F010 implements — and the detector was proven by injecting a marker and confirming it fired. |
| **V. Interaction Budget Verified** | **Yes, and this is the feature that tests it hardest — and the first to find the seam insufficient** | A terminal is the largest producer the control channel carries. FR-012 forbids output delaying interactive traffic and SC-006 measures it under 50 MiB, printed rather than asserted (A-NFR). The frame writer F004 built is **necessary and not sufficient**: it is a `Mutex<Box<dyn Write + Send>>` that holds the lock across a write and a flush, so it serialises correctly and orders by acquisition. §4.6 requires priority queueing, and the only implementation of it is `client/core`'s send queue — the client-to-engine direction. Engine to client there is none, so a 50 MiB build acquires that lock hundreds of times and a completion response queues behind it by arrival. F010 adds the engine-side queue. This is the failure that would compile, pass every functional criterion, and fail only SC-006. |
| **VI. Trust Boundaries Both Sides** | Yes | A task's working directory is resolved and contained by the engine through `ResolvedPath`, independently of the client (FR-003, §4.7). A task identity arriving from the client is untrusted: FR-031c requires attaching to be distinguishable from starting, so a client cannot silently start a second process under a live identity. |
| **VII. Every Feature Ships With Tests** | Yes | Chunking, ordering and backpressure are decidable without a process — the reader is fed bytes and asked what it emits. The mock daemon's `notify` directive carries server-originated frames under latency and loss. Real pty behaviour (does a process believe it is a terminal?) needs a real process, and those tests spawn one locally: no remote host, no network. |
| **VIII. Ports and Adapters** | Yes | `TaskRunner` is an outbound port — a capability, not a pty. The adapter naming the pseudo-terminal is one file, guarded as `inotify` is. Chunking, ordering and the retention bound are pure application code, fed bytes and told the time by the `Clock` port F004 added. Svelte components are inbound adapters; the terminal library lives in one of them. |

**Verdict before Phase 0: passes**, conditional on the Principle II amendments landing. They did.

### Re-check after Phase 2 design

The gate says re-check after design, and design moved four of these rows.

| Principle | Before | After | What moved it |
|---|---|---|---|
| **I. Design Fidelity** | Pass | **Pass, narrowed** | The row claimed the library's "default palette is a violation like any other raw value". Design found the system defines three of the sixteen colours a terminal renders, so satisfying that claim meant inventing thirteen — the act this principle exists to prevent. **A-TERMPALETTE** records the library's palette as the accepted source for what the system does not define, and SC-016 was narrowed to match. The principle holds; the row's absolute reading did not. |
| **II. One Source of Truth** | Blocking | **Pass** | Three methods added (`attach`, `list`, `workspace/close`), plus the encoding, arity, exit-shape, signal-typing and optionality statements the existing rows needed to be implementable. Nine §4.8 edits in total, and §4.6 rewritten to state its rule per direction. |
| **III. Decisions Recorded** | Pass | **Pass, two more** | **A-TASKSTREAM** (Phase 0) and **A-TASKEXEC** and **A-TERMPALETTE** (Phase 1 and 2). Appendix A held 40 records at that point. Each carries rejected alternatives and a reversal condition — A-TASKEXEC and A-TERMPALETTE did
not when this row first claimed it, and the claim was caught by analysis rather than by the gate
it was written into. Two more were added after: **A-STATE2**, superseding A-STATE because task
identities must join the client's durable payload and Principle III forbids editing a record in
place, and **A-WSCLOSE**, because three `workspace/close` decisions with stated rejected
alternatives had been recorded in a contract rather than in Appendix A. |
| **V. Interaction Budget** | Pass | **Pass, and the claim was wrong** | The row credited F004's frame writer as the seam this measures. It is the serialisation seam; priority is a different property that happens to live in the same place, and a mutex provides the first while precluding the second. F010 builds `send_queue.rs`. Recorded above. |
| **VIII. Ports and Adapters** | Pass | **Pass, sharpened** | The port splits into `TaskOutput` and `TaskControl` so a keystroke never waits behind a blocked read. Design found that split defeated by a single map-wide mutex over the task set, which satisfies every document as written — per-task locks are now stated, with control handles reachable without the map lock. |

Principles **IV**, **VI** and **VII** are unchanged: no live open items in any section F010 implements, containment still enforced engine-side independently of the client, and every decidable behaviour still testable with no remote host and no network.

**Verdict after design: passes.** Three things design found are foundational rather than story
work, and `tasks.md` must order them first: the id-less dispatch path, without which the engine
cannot receive a notification at all; the engine-side send queue, without which SC-006 fails while
everything else passes; and `WorkspaceRoots::deregister`, without which `workspace/close` cannot
close anything.

## Project Structure

### Documentation (this feature)

```text
specs/007-execution-terminals/
├── spec.md              # 43 FRs, 30 SCs, 5 stories, 7 decisions recorded in Appendix A
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── task-methods.md      # runTask, attach, list, writeStdin, resizePty, terminate, workspace/close
│   ├── task-events.md       # onStdout, onStderr, onExit
│   └── runner-port.md       # the engine's TaskRunner port
├── architecture.md      # Phase 2 output
├── design.md            # Phase 2 output
└── tasks.md             # Phase 3 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
protocol/src/
└── wire.rs                          # task params/results, output and exit payloads

engine/src/
├── domain/
│   └── task.rs                      # NEW: TaskId, Task, TaskSet, ExitStatus, OutputChunk
├── application/
│   ├── ports/
│   │   └── task_runner.rs           # NEW: TaskRunner port — a capability, not a pty
│   ├── output.rs                    # NEW: pure — chunking, ordering, the retention bound
│   └── use_cases/
│       └── task.rs                  # NEW: StartTask, AttachTask, WriteInput, Resize, Stop
└── adapters/
    ├── inbound/rpc.rs               # + the execution/* dispatch, + an id-less path so a
    │                                 #   notification can reach a handler at all
    └── outbound/
        ├── pty_runner.rs            # NEW: the ONLY file naming the pty mechanism
        ├── task_threads.rs          # NEW: a reader per task, writing through the send queue
        └── send_queue.rs            # NEW: engine-side priority queue (§4.6), A-PRI's
                                     #      `Interactive` ahead of `Background`; FrameWriter
                                     #      becomes its drain

client/core/src/
├── application/
│   ├── ports/task_provider.rs       # NEW: start, attach, write, resize, stop
│   └── use_cases/observe_task.rs    # NEW: chunk -> panel, exit -> panel
└── adapters/
    ├── inbound/task_notification.rs # NEW: notification -> use-case input
    ├── outbound/remote_tasks.rs     # NEW: the methods over the transport
    └── outbound/local_tasks.rs      # NEW: the local provider runner-port.md names

client/ui/lib/
├── terminal/
│   ├── TerminalPanel.svelte         # NEW: one per task, themed from the prototype
│   ├── panels.ts                    # NEW: the panel set, one instance per task (FR-026)
│   └── palette.ts                   # NEW: the three hues the system defines -> tokens;
│                                    #      the rest from the library (A-TERMPALETTE)
└── ds/layout-tokens.css             # + terminal font size, line height, cursor, padding

scripts/ds-sync.mjs                  # + extract the prototype's terminal dimensions

engine/tests/
└── pty_confinement.rs               # NEW: guards the one-file rule (Principle VIII)

tests/e2e/
└── terminal.spec.ts                 # NEW: run, type, resize, exit, reattach
```

Three additions to existing files that are easy to miss because they are not new files.
`application/ports/clock.rs` gains `Send + Sync` and a `sleep_until(deadline)` method: the
five-second escalation has no way to wait against a port whose only method is `now()`, and the
chunker reads the clock on N reader threads while the stop path reads it on the dispatch thread,
which a `Send`-not-`Sync` port cannot support. `FakeClock` moves from `Cell` to a `Mutex` plus a
`Condvar` so the wait stays deterministic. A single escalation thread owns the deadline set, so
the dispatch thread answers `terminate` immediately and never blocks — blocking it for five
seconds per stop is FR-012 failing through the mechanism meant to satisfy FR-017.
`application/ports/roots.rs` gains `deregister`: `WorkspaceRoots` offers `register` and `resolve`
only, so `workspace/close` cannot actually close anything and a second close cannot answer
`-32001` rather than succeeding silently. And `session.rs` gains a second environment variable
carrying the terminated task ids across the re-execution, without which A-TASKEXEC reports an
empty list and looks implemented (see that record).

**Structure Decision**: The existing three-crate workspace plus the webview, with no new crate.
The pty adapter is a file inside `engine`, not a crate: one implementation, one consumer, no
independent release cycle.

The rule this encodes, and the one F004 proved worth enforcing: **the pseudo-terminal is named in
exactly one file.** Everything that decides anything — how bytes are chunked, what order they
keep, when a producer is slowed, what is retained for an absent client — sits in application code
tested against an in-memory runner with no process at all. `engine/tests/pty_confinement.rs`
guards it, following `inotify_confinement.rs`, which caught its own author three times.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|--------------------------------------|
| Three new methods in §4.8 — `execution/attach`, `execution/list`, `workspace/close` | A-TASKLIFE makes a task outlive its connection, so a client must be able to reach one it did not start in this session. The catalogue has no such method | Making `runTask` idempotent — same id attaches rather than starts — needs no new method and makes the two outcomes indistinguishable at the call site, which is exactly what FR-031c forbids: a client racing its own reconnect would start a second process and be told it attached |
| A reader thread per task | The engine has no async runtime and a pty read blocks. One thread per task is the shape that costs nothing when idle | One thread multiplexing all tasks needs `poll` over a changing descriptor set, which is an event loop written by hand — the thing an async runtime would be for, minus the testing |
| `nix` as a new engine dependency | A pseudo-terminal, a process group and resource limits are all syscalls, and the engine has no libc binding at all — F004 hardcoded `ENOSPC` rather than add one | `portable-pty` is cross-platform and pulls far more for an engine that runs only on Linux. Raw `libc` with hand-written `unsafe` for four syscall families is more unsafe code than a narrow `nix` feature set, for no dependency saving worth the risk |
