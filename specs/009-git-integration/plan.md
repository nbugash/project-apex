# Implementation Plan: Git Integration

**Branch**: `feature/F011-git-integration` | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/009-git-integration/spec.md`

## Summary

The engine runs git and says what it found; the client stores that, colours the tree with it,
names the branch and draws gutters. Nothing here changes a repository.

Three things make this more than a pipe. The engine must notice an index change, which no
working-tree watcher sees, so it holds two watches of its own inside the repository's git
directory (A-GITWATCH). The status can be larger than a frame, so it is paged, which turns
"apply in one transaction" into something that commits on the *last* page rather than the first
(A-GITPAGE). And git state is keyed by path rather than by the tree's identity for a file,
because untracked and deleted paths have no tree entry — which is precisely when a developer
most wants to see them.

## Technical Context

**Language/Version**: Rust 1.75 (workspace MSRV) for `engine`, `protocol` and `client/core`;
TypeScript with Svelte 5 for the webview. No new language and no version change.

**Primary Dependencies**: None added. `git` is invoked as a subprocess on the host — the engine
already spawns processes for F010. Watching reuses `inotify 0.11` with `default-features = false`,
which F004 pinned that way after its defaults pulled tokio into a runtime-free binary.

**Storage**: The existing SQLite projection. The git status table F003 created is **re-keyed**
from the tree's file identity to (workspace, path); nothing has ever written to it, so this is a
schema edit and not a data migration. Schema version rises by one.

**Testing**: `cargo test` for the engine, protocol and client core; Vitest for pure webview
modules; WebdriverIO for end-to-end, in the live run because git status needs a real engine and
a real repository.

**Target Platform**: Linux host for the engine; Tauri client on Linux and macOS.

**Project Type**: Desktop application with a remote engine over SSH — unchanged.

**Performance Goals**: A change on the host is marked within 2 s (SC-001, SC-002). A modified
file's gutter appears within 250 ms at p99 over at least 100 samples, printed (SC-004, §1.4,
A-NFR). Rendering a folder of any size issues zero requests (SC-003).

**Constraints**: §4.1 caps a frame at 1 MiB, which is why status is paged. The engine is
synchronous and runtime-free; a status computation must not block the frame reader, so it runs
on its own thread. The engine's stderr is an 8 KiB classification buffer, not a log file, so a
status refresh logs nothing on the happy path. Git status must never touch cache validity (§5.3).

**Scale/Scope**: Repositories up to roughly 10^5 changed paths — a wholesale reformat or a
mid-rebase state — which is the case paging exists for. Ordinary working states are tens of paths.

### Fixed Quantities

Requirements that say "a stated quantity" are fixed here, with the reasoning, because a number
chosen during implementation is a number nobody reviewed.

| Quantity | Value | Why this value |
|---|---|---|
| Status page size | **1000 entries**, `limit` defaulting to and capped at it | §4.8 already fixes this for `workspace/readDirectory`, and status is the same shape of problem. A second number would be a second thing to justify and to keep in step. |
| Index-change coalescing | **100 ms** trailing edge | A-COALESCE's existing figure for repeated changes to one path, and the index *is* one path. Reusing it means one rule to understand rather than two that happen to agree. |
| Status computations in flight | **At most one**, with at most one more scheduled | A 100 ms trailing edge still admits many runs across a rebase that takes seconds, and a full status on a large repository is not free. A change arriving during a run marks the result dirty and schedules exactly one re-run, so a burst of any length costs two computations rather than N. |
| Freshness bound | **2 s** from change on the host to mark in the tree | SC-001 and SC-002. Nobody waits on git status the way they wait on a keystroke; the bound exists so that "eventually" cannot pass as a result. |
| Gutter budget | **250 ms** p99 over ≥100 samples, printed | §1.4's interaction budget. Opening a file is an interaction; the diff is part of what opening produces. |
| Git states | **5** — modified, untracked, staged, deleted, conflict | §4.8 fixes the set. Each maps to a design-system token; no token is created. |

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 2 design.*

| Principle | Verdict | Reasoning |
|---|---|---|
| **I. Design Fidelity** | **PASS, with one recorded deviation** | The tree's VCS marker column exists in the signed-off prototype — `ds-sync` extracted `--vk-tree-vcs-size` from it and F000 reserved the slot. The status bar branch does **not** exist in the prototype; searching its visible text for "branch" returns nothing. Recorded in spec.md's *Design deviations* rather than discovered later. Every colour comes from a design-system token. |
| **II. One Source of Truth** | **PASS** | The protocol is §4.8, amended there rather than restated here. The page size is `readDirectory`'s. The coalescing window is A-COALESCE's. Where this plan fixes something, *Fixed Quantities* says so and says why. |
| **III. Decisions Recorded First** | **PASS** | A-GITWATCH and A-GITPAGE were both written into Appendix A **before** this plan, because both bind features beyond F011 — the first amends A-IGNORE, which F004 and F013 rely on; the second is the pattern F013's search results will meet next. |
| **IV. Open Items Block** | **PASS** | Zero live `[OPEN:]` markers before Appendix A, verified by injecting one and confirming the detector fires — a detector that finds nothing and a detector that is broken produce identical output. |
| **V. Interaction Budget Verified** | **PASS** | SC-003 and SC-013 are counts, SC-004 is a printed p99 over ≥100 samples per A-NFR. Tree colouring reads the projection and issues nothing, which is what makes SC-003 a fact about the design rather than a hope. |
| **VI. Trust Boundaries Both Sides** | **PASS** | Paths arriving from git are workspace-relative and are contained on the client as well as the engine, because a path from a subprocess is untrusted input exactly as a path from the wire is. A status naming a workspace the client does not hold is discarded (FR-012). Nothing here writes a repository. |
| **VII. Every Feature Ships With Tests** | **PASS** | Unit: state precedence, the page accumulator, the coalescer. Integration: porcelain parsing against a real repository, the two watches firing on a real index write, paged status against a repository with more than one page of changes. End-to-end: each acceptance scenario with a real engine and a real repository. |
| **VIII. Ports and Adapters** | **PASS** | Git is an outbound port on the engine with a subprocess adapter, so parsing is testable against captured output with no repository. The git watch is a **separate** service from the workspace watcher rather than an exception inside its exclusion filter, which makes "these events never become file events" structural instead of a filter somebody can get wrong. |

**Gate result before Phase 0: PASS.** No violations, so *Complexity Tracking* records none.

### Re-check after Phase 2 design

Re-evaluated against the design as written, not against the intention that preceded it.

| Principle | Verdict | What changed or was confirmed |
|---|---|---|
| **I. Design Fidelity** | **PASS** | Confirmed against the prototype rather than assumed: the tree's VCS column is in it, the status-bar branch is not. `marker.ts` maps five states to existing tokens and creates none, so `lint:ds` has something to enforce. |
| **II. One Source of Truth** | **PASS** | `design.md` links entity fields to `data-model.md` instead of repeating them, and the state machine lives in one place. The one number this feature invented — the page size — is `readDirectory`'s, and the coalescing window is A-COALESCE's. |
| **III. Decisions Recorded First** | **PASS** | A-GITWATCH and A-GITPAGE predate this plan. Phase 0 then found something neither anticipated — a linked worktree's `.git` is a file, verified at 80 bytes — and the resolution was written into the spec's *Clarifications* before any design depended on it. |
| **IV. Open Items Block** | **PASS** | Unchanged; zero live markers, detector verified by injection. |
| **V. Interaction Budget Verified** | **PASS** | SC-004 is a printed p99 over ≥100 samples. SC-003's zero is now structural rather than aspirational: the tree reads the projection and the design gives it no path to a request. |
| **VI. Trust Boundaries Both Sides** | **PASS, and strengthened** | Phase 0 added a boundary the spec had not named: git is a **subprocess**, so its output is untrusted input. Paths are contained on the engine before emitting and again on the client, and a parse failure rejects the whole snapshot rather than applying part of it. |
| **VII. Every Feature Ships With Tests** | **PASS** | The ports exist so the hard parts are testable without a repository: parsing against captured output, coalescing against a fake clock, paging against a synthetic snapshot. `quickstart.md` carries eight mutations and eight negative checks with the condition that lets each fail. |
| **VIII. Ports and Adapters** | **PASS** | Git is a port with a subprocess adapter; watching is a second port. The git watch is a separate service, which is what makes "git events never become file events" a property of there being no code path rather than of a filter. |

**Gate result after Phase 2: PASS.** No violations; *Complexity Tracking* still records none.

## Project Structure

### Documentation (this feature)

```text
specs/009-git-integration/
├── spec.md              # What and why
├── plan.md              # This file
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── contracts/
│   └── git-status.md    # Phase 1
├── quickstart.md        # Phase 1
├── architecture.md      # Phase 2
├── design.md            # Phase 2
└── checklists/
    └── requirements.md
```

### Source Code (repository root)

```text
protocol/src/wire.rs                                  # git params and results, snake_case
engine/
├── src/application/ports/git.rs                      # the port: status, diff
├── src/application/use_cases/git_status.rs           # coalescing, one run in flight, paging
├── src/adapters/outbound/git_cli.rs                  # the subprocess adapter
├── src/adapters/outbound/git_watchers.rs            # watch + coalescer + pager -> notify
├── src/adapters/outbound/inotify_watcher.rs         # + the git watch: two watches, own
│                                                     #   inotify instance, no exclusion set
├── src/adapters/inbound/rpc.rs                       # dispatch arms
└── tests/                                            # parsing, watching, paging
client/core/
├── src/application/ports/git_provider.rs             # what the client reaches git through
├── src/application/use_cases/apply_git_status.rs     # page accumulation, one transaction
├── src/adapters/outbound/sqlite/schema.rs            # re-key, version bump
├── src/adapters/inbound/tauri_commands.rs            # git_status, git_file_diff
└── tests/
client/ui/lib/
├── git/status.svelte.ts                              # the projection the tree reads
├── git/marker.ts                                     # state to token and glyph, pure
├── git/gutter.ts                                     # coordinates to decorations, pure
├── shell/Window.svelte                               # starts the status subscription, once
├── workspace/FileTree.svelte                         # fills the reserved .vcs column
├── statusbar/StatusBar.svelte                        # the branch
└── editor/EditorPanel.svelte                         # gutter decorations
tests/unit/ tests/e2e/                                # Vitest and WebdriverIO
tests/e2e/wdio.conf.ts                                # git specs join the live run
```

**Structure decision**: Existing layout, existing boundaries. Git is one more outbound port on
the engine and one more projection on the client; no new crate and no new top-level directory.

## Complexity Tracking

No constitution gate was violated, so nothing is justified here. The one thing worth naming is a
deliberate *reduction*: the git watch is a second, separate watch service rather than a special
case inside the existing one. That is one more small component in exchange for a property —
git events cannot become file events — that holds by construction rather than by a filter
remaining correct as both features change.
