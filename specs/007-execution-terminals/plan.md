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

Unlike F004, §4.8 already defines the methods — all seven `execution/*` entries. One is missing
and this feature adds it: nothing in the catalogue attaches to a task that is already running,
which A-TASKLIFE made necessary the moment a task outlived its connection.

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
collectively exhausts is recorded as owed (A-TASKLIMIT).

**Scale/Scope**: A build emitting tens of megabytes over minutes, on a 16 vCPU / 128 GB instance
(§1). Concurrency is whatever the developer starts — two builds and a watcher is ordinary —
bounded by resource limits rather than a count.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Applies | How this feature satisfies it |
|---|---|---|
| **I. Design Fidelity** | **Yes — and the prototype is more specific than expected** | The prototype has a terminal dock, and it is the default one (`dock:'terminal'`). It specifies JetBrains Mono at 12.5px, line-height 1.6, padding `2px 12px 12px`, an accent-coloured shell prompt, and a 7×15px block cursor blinking on `vkpulse 1.1s steps(1,end) infinite`. Its title even switches on mode — `Terminal — build-01.euw1` against `Terminal — local`. **12.5px is a third font size**, distinct from `--vk-fs` and `--vk-code` at 13.5px, so `ds-sync` must extract it rather than a component inventing it. §8.3 names the library and Principle I names the appearance; the library is themed to the prototype, never the reverse, and its default palette is a violation like any other raw value. |
| **II. One Source of Truth** | **Yes — blocking** | §4.8 defines seven `execution/*` methods and **none attaches to a task already running**. `runTask` starts one; the other six address one the caller already knows. A-TASKLIFE made a task outlive its connection, so reattachment is now required (FR-031b) and undefined. The system specification MUST be amended before implementation — the sixth absence of this kind, after `workspace/register` and `workspace/watch`. |
| **III. Decisions Recorded** | **Already satisfied, and re-checked here** | Two decisions closing genuine alternatives are recorded: **A-TASKLIFE** (a task outlives its connection) and **A-TASKLIMIT** (bounded per process, not per tree). Both were made before this plan, both carry rejected alternatives and reversal conditions. A third is owed and named in Phase 0: the attach method's shape. |
| **IV. Open Items Block** | Passes | Checked at the cycle's step 2 with the bounded detector — no live `[OPEN: id]` in any section F010 implements — and the detector was proven by injecting a marker and confirming it fired. |
| **V. Interaction Budget Verified** | **Yes, and this is the feature that tests it hardest** | A terminal is the largest producer the control channel carries. FR-012 forbids output delaying interactive traffic and SC-006 measures it under 50 MiB, printed rather than asserted (A-NFR). The frame writer F004 built is the seam this measures, and F010 is the first feature to put real volume through it. |
| **VI. Trust Boundaries Both Sides** | Yes | A task's working directory is resolved and contained by the engine through `ResolvedPath`, independently of the client (FR-003, §4.7). A task identity arriving from the client is untrusted: FR-031c requires attaching to be distinguishable from starting, so a client cannot silently start a second process under a live identity. |
| **VII. Every Feature Ships With Tests** | Yes | Chunking, ordering and backpressure are decidable without a process — the reader is fed bytes and asked what it emits. The mock daemon's `notify` directive carries server-originated frames under latency and loss. Real pty behaviour (does a process believe it is a terminal?) needs a real process, and those tests spawn one locally: no remote host, no network. |
| **VIII. Ports and Adapters** | Yes | `TaskRunner` is an outbound port — a capability, not a pty. The adapter naming the pseudo-terminal is one file, guarded as `inotify` is. Chunking, ordering and the retention bound are pure application code, fed bytes and told the time by the `Clock` port F004 added. Svelte components are inbound adapters; the terminal library lives in one of them. |

**Verdict: passes, conditional on the Principle II amendment landing first.** That amendment is
Phase 0 work. No task may depend on attaching to a running task until §4.8 defines how.

## Project Structure

### Documentation (this feature)

```text
specs/007-execution-terminals/
├── spec.md              # 43 FRs, 28 SCs, 5 stories, 2 decisions recorded in Appendix A
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── task-methods.md      # runTask, attach, writeStdin, resizePty, terminate
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
    ├── inbound/rpc.rs               # + the execution/* dispatch
    └── outbound/
        ├── pty_runner.rs            # NEW: the ONLY file naming the pty mechanism
        └── task_threads.rs          # NEW: a reader per task, writing through FrameWriter

client/core/src/
├── application/
│   ├── ports/task_provider.rs       # NEW: start, attach, write, resize, stop
│   └── use_cases/observe_task.rs    # NEW: chunk -> panel, exit -> panel
└── adapters/
    ├── inbound/task_notification.rs # NEW: notification -> use-case input
    └── outbound/remote_tasks.rs     # NEW: the methods over the transport

client/ui/lib/
├── terminal/
│   ├── TerminalPanel.svelte         # NEW: one per task, themed from the prototype
│   └── palette.ts                   # NEW: ANSI names -> design-system tokens
└── ds/layout-tokens.css             # + terminal font size, line height, cursor, padding

scripts/ds-sync.mjs                  # + extract the prototype's terminal dimensions

tests/e2e/
└── terminal.spec.ts                 # NEW: run, type, resize, exit, reattach
```

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
| A new `execution/attach` method in §4.8 | A-TASKLIFE makes a task outlive its connection, so a client must be able to reach one it did not start in this session. The catalogue has no such method | Making `runTask` idempotent — same id attaches rather than starts — needs no new method and makes the two outcomes indistinguishable at the call site, which is exactly what FR-031c forbids: a client racing its own reconnect would start a second process and be told it attached |
| A reader thread per task | The engine has no async runtime and a pty read blocks. One thread per task is the shape that costs nothing when idle | One thread multiplexing all tasks needs `poll` over a changing descriptor set, which is an event loop written by hand — the thing an async runtime would be for, minus the testing |
| `nix` as a new engine dependency | A pseudo-terminal, a process group and resource limits are all syscalls, and the engine has no libc binding at all — F004 hardcoded `ENOSPC` rather than add one | `portable-pty` is cross-platform and pulls far more for an engine that runs only on Linux. Raw `libc` with hand-written `unsafe` for four syscall families is more unsafe code than a narrow `nix` feature set, for no dependency saving worth the risk |
