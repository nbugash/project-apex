# Implementation Plan: Offline Editing

**Branch**: `feature/F012-offline-editing` | **Date**: 2026-09-27 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/010-offline-editing/spec.md`

## Summary

Losing the connection leaves the editor writable. Saved edits are retained locally against the
hash of the content the client last confirmed with the host; on reconnection each file is merged
three ways — base, local, remote — fast-forwarding where the remote has not moved, combining
non-overlapping changes, and prompting only where a standard version-control merge would
conflict. Reading stays available from the cache throughout, and a bounded background prefetch
makes "what I can read offline" a deliberate set rather than an accident of what was opened.

The merge is `diffy`, chosen because it was **measured** against `git merge-file` and disagreed
on none of eight cases, including the two that decide the design (adjacent lines conflict; two
lines apart merge). See [research.md](./research.md).

## Technical Context

**Language/Version**: Rust 1.75 (MSRV, unchanged), TypeScript 5 with Svelte 5 in the webview

**Primary Dependencies**: existing — `rusqlite` (bundled SQLite), `zstd`, `sha2`, `tokio`,
`tauri` 2, `apex-protocol`. **New: `diffy` 0.4** for the three-way merge, the only dependency
this feature adds. Verified to build under MSRV 1.75 (0.5 requires 1.85 and is not used).

**Storage**: the existing client projection (§5.2), migrated to **schema version 4**, adding a
`pending_edits` table keyed by `(workspace_id, relative_path)` and holding the offline content,
**the base content** and its hash. The base is stored rather than referenced because a three-way
merge needs the base text and `file_contents` is evictable — analyze run 2 found that a referenced
base would be gone on exactly the path the merge exists for. Engine stores nothing new.

**Testing**: `cargo test` for engine and client core; `vitest` for pure webview logic; WebdriverIO
against a real engine and a real workspace for the end-to-end scenarios, in the **live** suite
because reconnection needs an engine to disconnect from.

**Target Platform**: Linux and macOS desktop client; Linux remote engine. Offline behaviour is a
property of the client, so the engine half is limited to one new read-only method.

**Project Type**: desktop application with a remote daemon — the established shape.

**Performance Goals**: a cached file opens offline in under 200 ms (SC-007); offline path search
over 50,000 cached paths returns in under 1 second (SC-008); reconciling 100 files completes in
under 10 seconds (SC-011); interactive actions during prefetch stay within 10% of their idle
latency (SC-009).

**Constraints**: the merge runs on the client, which cannot assume git is installed — offline is
exactly when spawning processes is least appropriate. Prefetch is background priority (§4.6) and
must never delay interactive traffic. Nothing here may change what makes cached content valid
(§5.3): validity is a hash comparison and stays one.

**Scale/Scope**: 39 functional requirements, 15 success criteria, 5 user stories, 29 acceptance
scenarios. One new protocol method, one schema migration, one new client dependency.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Gate | Result before Phase 0 |
|---|---|---|
| I. Design Fidelity | Every new surface comes from design-system tokens; no invented values | **PASS** — the offline indicator extends the existing status bar, and the conflict interface is new and must be built from tokens. Flagged below as a deviation to record, because the prototype has no conflict screen |
| II. One Source of Truth | No second authority for a fact the system already holds | **PASS** — offline state comes from F001's published connection state (FR-001); the base hash is the protocol's existing `baseSha256`; validity stays §5.3's hash comparison |
| III. Decisions Recorded First | Alternatives closed are recorded before the code | **PASS** — research.md records six, three of which bind beyond F012 and go to Appendix A before implementation |
| IV. Open Items Block | No live `[OPEN:]` marker in the sections this feature implements | **PASS** — checked at cycle preflight; none live before Appendix A |
| V. Interaction Budget Verified | Anything on the interaction path ships with a failing-when-exceeded measurement | **PASS** — SC-007, SC-008, SC-009 and SC-011 are measurements, and quickstart records them as numbers |
| VI. Trust Boundaries Both Sides | Paths validated on both sides | **PASS** — pending edits are keyed by a path the client re-validates on read, and the write path already validates on both sides |
| VII. Every Feature Ships With Tests | Every acceptance scenario has an automated test | **PASS** — 29 scenarios; the plan allocates each to a suite in quickstart.md |
| VIII. Ports and Adapters | Use cases orchestrate and return plain data; adapters hold I/O | **PASS** — the merge is a pure function over three strings, the reconciler is a use case, and `diffy` is named only inside one adapter, as `inotify` is |

**One deviation to record.** The conflict interface does not exist in the signed-off prototype.
This is the same shape as F011's branch indicator, which spec.md recorded as a deviation and the
design system absorbed without incident. It is recorded here and in spec.md rather than decided
silently; the interface must be built from tokens so a designer can move it without unpicking an
improvised value.

**Re-check after Phase 2 design: PASS.** The design keeps `diffy` inside
`client/core/src/adapters/outbound/text_merge.rs` and the reconciler free of it, so Principle
VIII holds as written; a confinement test enforces it the way `inotify_confinement.rs` does. No
principle changed status between the two evaluations.

## Project Structure

### Documentation (this feature)

```text
specs/010-offline-editing/
├── plan.md              # This file
├── research.md          # Phase 0: six decisions, one of them measured
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/           # Phase 1
│   ├── recently-changed.md
│   └── offline-commands.md
├── architecture.md      # Phase 2
├── design.md            # Phase 2
└── tasks.md             # Phase 3 (/speckit-tasks, not created here)
```

### Source Code (repository root)

```text
protocol/src/
└── wire.rs                                     # + RecentlyChangedParams/Result

engine/src/
├── adapters/outbound/git_cli.rs                # + recently_changed via `git log --name-only`
├── adapters/inbound/rpc.rs                     # + git/recentlyChanged arm
└── application/ports/git.rs                    # + recently_changed on the Git port

client/core/src/
├── adapters/outbound/
│   ├── sqlite/schema.rs                        # + V4: pending_edits
│   ├── sqlite/migrate.rs                       # + 4 => schema::V4
│   ├── sqlite/mod.rs                           # + pending-edit reads and writes
│   └── text_merge.rs                           # the ONLY file naming `diffy`
├── application/
│   ├── ports/
│   │   ├── workspace_cache.rs                  # + pending-edit operations
│   │   └── text_merge.rs                       # the merge port: three strings in, outcome out
│   └── use_cases/
│       ├── retain_edit.rs                      # save offline -> pending edit
│       ├── reconcile.rs                        # on reconnect: per file, merge or prompt
│       └── prefetch.rs                         # bounded, never evicts
└── adapters/inbound/tauri_commands.rs          # + offline_status, conflicts_list, conflict_resolve

client/ui/lib/
├── offline/
│   ├── state.svelte.ts                         # offline projection, from the connection state
│   └── conflicts.svelte.ts                     # outstanding conflicts
├── offline/ConflictPanel.svelte                # the deviation recorded above
└── editor/EditorPanel.svelte                   # writable offline; "held locally" indicator

engine/tests/                                   # recently_changed parsing and degradation
client/core/tests/                              # merge agreement, retention, reconciliation
tests/unit/                                     # pure webview logic
tests/e2e/                                      # live suite: the 29 scenarios that need an engine
```

**Structure Decision.** The established layout is unchanged. Everything new on the client sits in
the three places the architecture already has for it — a port, an adapter, a use case — and the
single new engine capability follows F011's git path exactly, because it *is* the git path: the
same `Git` port, the same `GitCli` adapter, the same dispatch shape.

The one structural rule this feature adds is the confinement of `diffy` to `text_merge.rs`. It is
the same rule `inotify_confinement.rs` enforces for the watcher, for the same reason: if the merge
library may be named anywhere, then anywhere may decide what a conflict is, and the conflict
boundary is the property SC-006b pins down.

## Complexity Tracking

| Addition | Why it is necessary | What was rejected |
|---|---|---|
| `diffy` dependency | SC-006b requires agreement with a standard merge tool; hand-writing one makes agreement an intention rather than a property | Writing the merge; `git2`; shelling out to `git merge-file` — all in research.md |
| New protocol method `git/recentlyChanged` | Nothing in §4.8 reports history, and prefetch's value is files the developer has *not* opened | Prefetching only opened files plus manifests, which drops half of FR-029 |
| Schema version 4 | A pending edit outlives its cache entry and can exist with no cache entry at all | Columns on `file_contents`; a directory of files |
| Two compressed copies per pending edit | The base content must be stored, not referenced: the merge needs its text and the cache's copy is evicted and overwritten on its own terms | Pinning the cached blob against eviction, which couples two tables' lifetimes that A-PENDING separated, and still loses to a refetch. Accepted because pending edits are few and short-lived, so the cost is bounded by how much unreconciled offline work exists |
| Conflict interface not in the prototype | FR-021 requires prompting, and a prompt needs somewhere to happen | Recorded as a deviation rather than resolved silently |

**Scope note, carried from spec.md.** This cycle builds all five subfeatures where A-OFFLINE
estimates roughly three features' worth. The reviewer decided that deliberately and the cost is
recorded in spec.md's `## Clarifications`: the merge semantics arrive at the end of the largest
change set the project has produced. The mitigation available to planning is to keep the merge
independently reviewable — its own port, its own adapter, its own confinement test, its own
agreement suite against `git merge-file` — so that it can be read without reading the rest.
