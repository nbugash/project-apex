# Implementation Plan: File Watch Sync

**Branch**: `feature/F004-file-watch-sync` | **Date**: 2026-09-24 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/006-file-watch-sync/spec.md`

## Summary

A developer working against a remote workspace currently learns that a file changed only by
asking. F004 closes that gap: the engine observes the workspace, the client is told what
moved, and the projection F003 built is corrected rather than rebuilt.

The approach has three parts. The engine watches **only what the developer has opened** —
expanded folders, the directories holding open tabs, and their ancestors — so watch cost
scales with attention rather than with repository size. A pure coalescing stage between the
raw filesystem stream and the wire collapses repeated writes and converts a flood into one
wholesale invalidation, with both thresholds fixed as numbers rather than judgements. The
client treats every arriving path as untrusted, re-validates containment itself, and applies
events to the SQLite projection without discarding cached content — a blob still proves
itself by hash, exactly as it did before the event arrived.

Two protocol methods do not exist yet and this feature adds them. That is the same shape as
F003, which found `workspace/register` missing from §4.8 and amended the system
specification before building against it.

## Technical Context

**Language/Version**: Rust with MSRV **1.75**, declared identically by `protocol`, `engine` and
`client/core`; TypeScript 5.x with Svelte 5 in the webview.

**Primary Dependencies**: `inotify` 0.11 in the engine — the one new dependency, and the only
file allowed to name it. **No async runtime is added.** `engine/Cargo.toml` records that the
engine "stays synchronous and runtime-free, because it is embedded in the client and
transferred on every first connect", so the watcher runs on a dedicated OS thread reading
inotify's file descriptor and hands events to the session loop over a `std::sync::mpsc`
channel. Client-side: `rusqlite` with `bundled` and `serde`/`serde_json`, all already present.
**No new client dependency.**

**Storage**: The existing client-side SQLite projection from F003 — `files` tree rows, the
FTS5 external-content index with its three synchronisation triggers, and the zstd content
blobs. F004 adds two columns and narrows one FTS5 trigger; no new tables. Engine-side watch state is in memory and
dies with the process, like the workspace registry.

**Testing**: `cargo test --workspace` for Rust at all three levels of Principle VII — unit
against in-memory fakes, integration across the protocol boundary, and the mock SSH daemon
from F001; `npm run test:unit` for TypeScript; WebdriverIO end-to-end specs under
`tests/e2e/`. Every level runs with no remote host and no network (A-TEST, FR-028).

**Target Platform**: Engine on Linux (EC2); client on macOS, Linux and Windows. The watch
adapter is Linux-only by construction — it is an outbound adapter behind a port, so the
client-side and domain tests never touch it. Local mode on macOS and Windows is F004's
`watch()` returning `Unsupported`, not a second native watcher; see research.md.

**Project Type**: Desktop application with a remote daemon — the existing Cargo workspace
(`protocol`, `engine`, `client/core`) plus the Svelte webview.

**Performance Goals**: A change to an open tab is reflected within **2 seconds** of the write
landing on the host (SC-001, a value this specification chose — see Assumptions). Watch
establishment for one folder completes within the §1.4 interaction budget, because expanding
a folder is a developer-initiated interaction.

**Constraints**: Event delivery must not delay interactive traffic on the control channel
(FR-016, §4.6, Principle V). No frame may exceed 1 MiB (§4.1); the bulk threshold is chosen
so an event batch cannot approach it. Watching must degrade rather than fail when the host's
capacity is exhausted (FR-005a). The client sets no OS watches in remote mode (§10.3).

**Scale/Scope**: A repository of a hundred thousand files with ten folders expanded holds
watches for those ten, their ancestors and any open tabs outside them — tens of watches, not
tens of thousands. Bursts of ten thousand changes are an exercised case (SC-005).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Applies | How this feature satisfies it |
|---|---|---|
| **I. Design Fidelity** | Yes | The tree already renders from `mockups/Apex IDE (standalone).html` tokens; F004 changes its contents, not its appearance. A stale-tree indication and a changed-tab marker are new visual states and MUST come from existing design tokens — no raw hex, no raw pixels. If the prototype has no state for them, that is a deviation needing written designer approval recorded here before implementation, not an invented style. |
| **II. One Source of Truth** | **Yes — blocking** | §4.8 defines `workspace/onFileEvent` and `workspace/invalidateAll` as notifications and **no method to start or stop watching**, while §6.1 declares `watch()` on the provider trait and §10.3 describes the engine watching from registration. The spec's first clarification resolved this: watching is scoped to what the client has open, which the engine cannot infer. The system specification MUST be amended before implementation — see Phase 0, *Protocol additions*. |
| **III. Decisions Recorded** | Yes | Four decisions close genuine alternatives and need Appendix A records before code: the coalescing window, the bulk threshold, the watch-scope model, and the unproven-content representation. Listed in Phase 0 with their reversal conditions. |
| **IV. Open Items Block** | Yes | `grep -c 'OPEN: ' project-apex-predator.md` must be zero for the sections F004 touches (§4.8, §10.3, §10.4, §5.2). Checked at Phase 0 before any artifact is written. |
| **V. Interaction Budget Verified** | Yes | Two obligations. Expanding a folder starts a watch and MUST stay inside §1.4 — measured, printed, not asserted (A-NFR). Event delivery MUST NOT delay interactive traffic, which is why coalescing and the bulk threshold exist at all: an unbounded event stream on the control channel is precisely the failure Principle V forbids. |
| **VI. Trust Boundaries Both Sides** | Yes | FR-014 requires the client to refuse any event path escaping the workspace root **independently of anything the engine checked**. The engine resolves and contains every watched path through the existing two-stage `ResolvedPath` (`engine/src/domain/path.rs`); the client re-validates arriving paths through its own containment in `local_workspace.rs`. Both sides, as the principle requires. |
| **VII. Every Feature Ships With Tests** | Yes | Coalescing and the bulk threshold are pure functions over a fake clock, so the volume requirements are unit-testable without a filesystem. Integration covers the protocol round trip; the mock SSH daemon covers delivery under 250 ms RTT and 5% loss; end-to-end covers the tree updating and the tab being marked. |
| **VIII. Ports and Adapters** | Yes | `FileWatcher` is an outbound port in the engine — a capability, not `inotify`. `Clock` becomes an engine port too, because coalescing needs time and the domain may not read it. The coalescer itself is pure application code with no `inotify`, no threads and no `serde` in scope — it is fed events and told the time. On the client the notification arrives at an inbound adapter that translates it into use-case input. |

**Verdict at Phase 0: passes, conditional on the Principle II amendment landing first.**

### Re-evaluated after Phase 2 design

| Principle | Status | What changed |
|---|---|---|
| **I. Design Fidelity** | **One open question, narrowed** | The prototype answers half of it. It already renders stale as **dimming** — "Dimmed rows are stale … They are never waited on." — so a stale tree region follows an existing treatment and needs no new token and no deviation. It does **not** answer the other half: the 6px tab dot is already bound to `t.dirty → var(--color-accent)`, meaning *unsaved local changes*. FR-023a needs a marker for *changed on the host*, and reusing one dot for two meanings is ambiguous rather than faithful. That is a narrow designer question — one affordance, one decision — and Principle I requires the answer in writing here before implementation, not a style invented during it |
| **II. One Source of Truth** | **Now passes** | The amendment landed. It was larger than Phase 0 estimated: six edits, not two. §4.8 gained the two method rows, a batched `onFileEvent` carrying entry metadata, and the `event` vocabulary; §6.1's normative `watch()` was corrected and `WatchHandle` removed; §10.3 was narrowed; §5.2's FTS5 update trigger was narrowed. Four of the six were found by writing the contracts *against* the specification rather than against the plan |
| **III. Decisions Recorded** | **Now passes** | Four records written: A-WATCHSCOPE, A-COALESCE, A-UNPROVEN, A-WATCHLOCAL. Appendix A goes from 31 to 35 entries. Kernel queue overflow was absorbed into A-COALESCE rather than taking a fifth identity |
| **IV. Open Items Block** | Passes, unchanged | Verified at Phase 0 and not disturbed: all nineteen `OPEN:` occurrences are resolution records, and Appendix B states everything was resolved on 2026-09-23 |
| **V. Interaction Budget Verified** | Passes, with a sharper target | Phase 1 found that the writer this feature's measurement applies to **does not exist**: `rpc.rs` returns frames and the session loop writes them, unsynchronised, because there has never been a second writer. F004 introduces it. FR-016's measurement is therefore a measurement of a seam this feature builds, which makes it a first-class task rather than an assertion about existing code |
| **VI. Trust Boundaries Both Sides** | Passes, unchanged | Engine-side containment through `ResolvedPath`; client-side re-validation before any projection write. `-32002` deliberately fails the whole call rather than degrading to a per-path refusal, because §4.7 is normative for every method and a per-path boundary check would be advisory |
| **VII. Every Feature Ships With Tests** | Passes, with one gap named | Coalescing, the bulk rule and rename pairing are pure over a fake clock. Two limits are recorded rather than hidden: SC-003 cannot be fully verified because there is no indexer to compare exclusion sets with, so only the structural half is provable now; and the mock SSH daemon has no way to originate a frame, which F004 fixes with a directive carrying a caller-supplied opaque frame |
| **VIII. Ports and Adapters** | Passes | `FileWatcher` and `Clock` are capabilities; `inotify` is confined to one file; the coalescer is pure. `held()` was added to the port so the watch-count criteria assert through the seam instead of reading `/proc`. `frame_writer.rs` is a new outbound adapter, not a global |

**Verdict after design: passes, with one Principle I question owed in writing** — whether the
changed-on-host tab marker may share the dirty dot's affordance or needs its own. Everything else
is resolved and recorded. No task may begin on the tab marker until that answer exists; every
other task is unblocked.

## Project Structure

### Documentation (this feature)

```text
specs/006-file-watch-sync/
├── spec.md              # 38 FRs, 22 SCs, 4 clarifications
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── watch-methods.md     # workspace/watch, workspace/unwatch
│   ├── file-events.md       # workspace/onFileEvent, workspace/invalidateAll
│   └── watcher-port.md      # the engine's FileWatcher port
├── architecture.md      # Phase 2 output
├── design.md            # Phase 2 output
└── tasks.md             # Phase 3 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
protocol/src/
└── wire.rs                          # watch params/results, event payloads

engine/src/
├── domain/
│   ├── path.rs                      # existing; ResolvedPath reused unchanged
│   └── watch.rs                     # NEW: WatchSet, RawEvent, FileEvent, EventKind
├── application/
│   ├── ports/
│   │   ├── file_watcher.rs          # NEW: FileWatcher port — a capability
│   │   └── clock.rs                 # NEW: Clock port — coalescing needs time
│   ├── coalescer.rs                 # NEW: pure; window, rename pairing, bulk threshold
│   └── use_cases/
│       └── watch.rs                 # NEW: SetWatchedPaths, ReleaseWatches
└── adapters/
    ├── inbound/rpc.rs               # workspace/watch, workspace/unwatch dispatch
    └── outbound/
        └── inotify_watcher.rs       # NEW: the only file that names inotify

client/core/src/
├── application/
│   ├── ports/
│   │   ├── workspace_provider.rs    # watch() seam F003 left; becomes real
│   │   └── workspace_cache.rs       # mark_unproven, rename_subtree
│   └── use_cases/
│       └── apply_file_event.rs      # NEW: event -> projection, containment first
├── adapters/
│   ├── inbound/
│   │   └── file_event_notification.rs   # NEW: notification -> use-case input
│   └── outbound/
│       ├── sqlite/{schema.rs,migrate.rs}  # user_version bump, two columns
│       ├── remote_workspace.rs      # watch/unwatch over the transport
│       └── local_workspace.rs       # local-mode watching, own containment
└── composition_workspace.rs         # wiring; no new global state

client/ui/lib/
├── workspace/
│   ├── FileTree.svelte              # stale marking (dimmed), in-place row updates
│   ├── tree.svelte.ts               # WorkspaceTree: applies events to the rendered tree
│   └── watched.svelte.ts            # NEW: derives the watched set from expansion + open tabs
├── tabs/TabStrip.svelte             # changed-tab marking (FR-023a)
└── statusbar/StatusBar.svelte       # "changes are not being reported" (FR-025)

tests/
├── unit/                            # webview unit specs live here, not under client/ui/
└── e2e/
    └── file-watch.spec.ts           # NEW: tree updates, tab marked, no focus change
```

**Structure Decision**: The existing three-crate Cargo workspace plus the Svelte webview, with
no new crate. The watcher is an adapter inside `engine`, not a crate of its own: it has one
implementation, one consumer and no independent release cycle, and a fourth crate would buy
compile-time isolation this feature does not need. `protocol` gains only wire types, keeping
its role as the shared vocabulary of the two binaries.

The one structural rule worth stating: **`inotify` appears in exactly one file**,
`engine/src/adapters/outbound/inotify_watcher.rs`. Everything that decides anything — which
paths are watched, when events collapse, when a flood becomes an invalidation — sits in
application code tested against a fake watcher and a fake clock. That is what makes FR-012,
FR-015 and SC-005 testable without a filesystem, and it is Principle VIII doing real work
rather than being observed.

## Complexity Tracking

> Filled only where the Constitution Check needs a justification.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Two new protocol methods (`workspace/watch`, `workspace/unwatch`) rather than reusing registration | The engine cannot infer which folders are expanded or which files are open, and the first clarification makes watch scope follow exactly that. Without a way to say so, the engine must watch everything — the cost FR-003 exists to avoid | Watching the whole workspace from `workspace/register` needs no new method and is what §10.3 reads like today. Rejected because it exhausts host watch capacity on a large repository, which is the failure the clarification was asked about |
| A `Clock` port in the engine where none existed | Coalescing and the bulk window are time-dependent, and Principle VIII names the clock an outbound port explicitly. A fake clock is what makes "a thousand writes in one second" a unit test rather than a sleep | Reading `Instant::now()` inside the coalescer needs no port and makes every volume test wall-clock-dependent and slow. Rejected on Principle VII and VIII together |
