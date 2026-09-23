# Implementation Plan: Workspace Cache

**Branch**: `feature/F003-workspace-cache` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/005-workspace-cache/spec.md`

## Summary

Deliver the read path of the workspace: one provider interface that hides whether content came
from disk or from the engine, a SQLite projection that answers the sidebar without touching the
network, and the first workspace methods the engine has ever implemented.

Three planning insights the specification does not state, because each is a "how" rather than a
"what":

**This feature makes the engine hexagonal.** F002 built the engine as a stdio loop with a `match`
on the method name — correct for three session methods with no logic behind them. F003 gives the
engine its first real behaviour: path canonicalisation, directory paging, ranged reads. Principle
VIII requires both codebases to be ports and adapters, and the moment there is logic to keep out
of the adapter is the moment that requirement starts to cost something. Deferring it means F004's
watcher and F013's search extend a shape the constitution already rejects.

**Caching is not an adapter.** The obvious reading of "the UI never learns which is active" is a
`CachingWorkspaceProvider` decorator sitting in `adapters/`. But the rules it applies — a hash
match serves from cache, a miss fetches, a disconnection changes what may be shown, every hit
records an access — are business rules, and Principle VIII puts business rules in the application
layer. The caching provider therefore lives in `application/` and *implements* the port it also
consumes. That shape is what lets the whole of US2, US4 and US5 be tested against an in-memory
fake cache and a fake engine, with no SQLite file and no process.

**The §1.4 sidebar target is a storage decision, not an optimisation.** "Folder expand (cached)
< 1 ms" is a p99 measured at the interface boundary (A-NFR), and the work inside that millisecond
is one indexed SQLite query plus whatever the runtime does to get to it. That rules out crossing
an `await` point that can park behind unrelated work, which is why the read path's concurrency
shape is decided in research.md rather than discovered in a profiler.

## Technical Context

**Language/Version**: Rust 1.75+ (edition 2021) for the client core and the engine. The interface
layer is unchanged Svelte 5 and TypeScript 5.x, touched only for the two states this feature has
to publish (verification in progress, maintenance running).

**Primary Dependencies**: Existing — `tokio`, `serde`, `serde_json`, `thiserror`, `uuid`,
`apex-protocol`. New, each selected in research.md — `rusqlite` (bundled, FTS5) for the
projection, `zstd` for content blobs, `sha2` for content hashing on both ends, `async-trait` for
the one port that must be `dyn`. The engine gains `sha2` and nothing else: it stays synchronous
and runtime-free, which research.md argues rather than assumes.

**Storage**: SQLite, one database file per installation, holding the canonical §5.2 schema for
every registered workspace. WAL, `synchronous = NORMAL`, foreign keys on, FTS5 for offline path
search. Schema version is `PRAGMA user_version`. Content blobs are Zstd level 3 in
`file_contents`, hashed over the decompressed bytes so the hash compares directly with the
engine's.

**Testing**: `cargo test` at three levels per A-TEST. Unit — path containment, cache validity,
page cursors, retention arithmetic, all against in-memory fakes. Integration — the schema against
a real database file in a temp directory, and the engine's workspace methods against a real
filesystem tree, with the engine spawned as a local child process exactly as F002 spawns it.
End-to-end — WebdriverIO on Linux per A-E2E for the two published states. The opt-in `sshd` suite
gains the bulk-read invocation, which is the one thing a local spawn cannot prove.

**Target Platform**: Client on Linux and macOS desktop. Engine on Linux x86-64 and ARM64.

**Project Type**: Desktop application plus a remote daemon, with a local database between them.

**Performance Goals**: Cached folder expand < 1 ms, uncached < 250 ms, both p99 over at least 100
samples at the interface/transport boundary with harness delay excluded (§1.4, A-NFR, SC-004c).
Compressed content at most half the size of what it represents, measured over a real source tree
and printed (SC-010). A migration publishes its state at least once per second (SC-013a).

**Constraints**: Never serve unverified content while connected (FR-021a). Never wait
indefinitely for a confirmation (FR-021c). Never read a projection with a schema it was not
written for, including mid-migration (FR-018c). Never evict while a workspace is open (FR-026a).
Never let a caching failure fail the read that prompted it (FR-034). Every behaviour verifiable
with no remote host and no network (FR-035).

**Screenshot convention**: End-to-end screenshots go to `reports/screenshots/${OS}/${FEATURE}/`
where `FEATURE` is the **feature map identity** — `F003` — not the spec directory number. The two
diverge and the map identity is the one guaranteed never to be renumbered. The segment is derived
from the git branch by `tests/e2e/wdio.conf.ts`, which F002 already made recursive over
subdirectories.

**Scale/Scope**: Repositories of hundreds of thousands of files, of which a session touches
hundreds. Directory listings paged at 1000 entries. Files up to the cache eligibility cap chosen
in research.md; larger ones are read but not cached.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|---|---|---|
| **I. Design Fidelity** | Applies | Two new surfaces: a verification indicator (FR-021b) and a maintenance state (FR-018a). Both are built from design system tokens and pass the adherence lint. Neither introduces a new visual language — the status bar already has a state vocabulary from F001 and F002, and these extend `presentation.ts` rather than inventing a parallel one. |
| **II. One Source of Truth** | **Applies — three amendments owed** | Every normative value here is quoted from §1.4, §4.1, §4.6, §4.7, §4.8, §5.1-5.6, §6.1-6.2, §10.1 or a recorded decision. **Three** things this feature needs are *absent* from the source of truth rather than contradicted by it. `workspace/readDirectory` has no pagination (FR-024 says so explicitly). §5.2's external-content FTS5 table declares no synchronisation triggers, so as written it returns nothing, silently, forever. And **there is no method by which the engine ever learns what a `workspaceId` means** — §15.4 step 3 says to register a workspace and §4.4 reserves `-32001` for one that is not registered, but §4.8 defines no `workspace/register`, which makes every other workspace method unusable. All three MUST land in `project-apex-predator.md` before the code that depends on them. Recorded as implementation obligations so `/speckit-tasks` carries them. |
| **III. Decisions Recorded** | Pass | Phase 0 records eleven decisions before any implementation. Three bind later features and are marked for promotion to Appendix A: the bulk-read threshold, the cache eligibility cap and the confirmation limit. |
| **IV. Open Items Block** | **Pass** | Verified mechanically, not by eye: `awk '/^# Appendix A/{exit} /\[OPEN: [A-Z][A-Za-z0-9-]*\]/{print NR": "$0}' project-apex-predator.md` returns nothing. All fifteen open items were resolved on 2026-09-23. A plain `grep -n 'OPEN: '` returns nineteen matches, every one of them in Appendix A's own resolution notes, which is why the gate is anchored. |
| **V. Interaction Budget Verified** | **Applies, centrally** | This feature owns two of the six rows in §1.4's table. SC-004c measures both at p99 with the value printed (A-NFR). Rule 2 of §1.5 and rule 2 of §4.6 — no bulk payload on the control channel — is the reason the bulk threshold exists at all rather than being a size heuristic. |
| **VI. Trust Boundaries Both Sides** | **Applies, and lands here** | F002's plan recorded that the engine had no workspace method to apply path containment to, and that the obligation fell to F003. It does. The engine canonicalises and asserts containment independently of the client (FR-005, FR-006), rejects with `-32002`, and does not distinguish a refusal from a miss (FR-007). The client validates too, and the design does not let the client's check be load-bearing (FR-008). |
| **VII. Every Feature Ships With Tests** | Applies | All three levels have surface. Unit: containment, validity, cursors, retention. Integration: the schema against a real file, the engine against a real tree. End to end: every acceptance scenario in the spec. No level is omitted, so no justification is owed. |
| **VIII. Ports and Adapters** | **Applies to both binaries** | Client ports introduced: `WorkspaceCache`, `BulkTransfer`, `Clock`. Client port consumed: `RequestTransport` (F001), `ConnectionStatusSource` (F001). The `WorkspaceProvider` of §6.1 is both — an outbound port with a remote adapter, and the interface the application's caching layer implements. Engine ports introduced: `FileSystem`, `WorkspaceRoots`. Engine adapters: `StdFileSystem`, and the JSON-RPC dispatch becomes an inbound adapter rather than a `match` in `main`. |

**Gate result: PASS with three recorded obligations.** None is a violation to justify — all three are
gaps in the system specification that this plan closes by amending it, which is what Principle II
prescribes. Complexity Tracking below is empty.

### Re-evaluated after Phase 2 design

The design settled four things the pre-check could not have anticipated. Each was checked against
the constitution rather than waved through.

**The caching provider sits in the application layer and implements an outbound port.** A type
that implements a port while consuming one looks like a layering smell. It is not: `CachedWorkspace`
holds ports (`WorkspaceCache`, an inner `WorkspaceProvider`, `Clock`, `ConnectionStatusSource`),
contains only rules, and imports no framework type. The alternative — a decorator in `adapters/` —
would put cache validity, staleness marking and access recording in an adapter, which Principle
VIII forbids for exactly the reason that would then apply: none of it would be testable without
SQLite. Principle VIII is satisfied; research.md records it under "Where cache policy lives".

**`spawn_blocking` at the SQLite boundary is not a process hop.** Principle VIII's last clause
forbids adding a serialization or process boundary to the interaction path, and Principle V wins
where they appear to conflict. A blocking-pool handoff is neither: nothing is serialised, no
process is crossed, and the measured cost is a thread wake. Principle V's actual demand — the
1 ms p99 — is what research.md's concurrency decision is chosen against, and SC-004c is what
proves it rather than this paragraph.

**The engine is restructured, which is scope this feature did not ask for.** F003's specification
says nothing about how the engine is organised. Principle VIII does, and it applies to the engine
"from this feature onward" in F002's words. The restructure is three files moved and one trait
introduced, not a rewrite, and it is confined to the engine crate. Recorded here because a
reviewer is entitled to ask why a cache feature touched `engine/src/main.rs`.

**The full §5.2 schema is created at version 1, including `git_status`, which nothing writes yet.**
This looks like speculative work that the YAGNI rule would delete. It is not speculative: §5.2 is
*canonical* under A-B5, the table is specified rather than anticipated, and creating it now costs
one `CREATE TABLE` while creating it later costs a migration, a migration test and a second
schema version for every installation in existence. Recorded under "What the first schema version
contains".

No new violations. The third amendment obligation was found by the Phase 1 reconciliation pass and is recorded above rather than in the pre-check, because the pre-check could not have seen it. Gate still **PASS**.

## Project Structure

### Documentation (this feature)

```text
specs/005-workspace-cache/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── workspace-methods.md    # the §4.8 read methods, incl. the pagination amendment
│   ├── provider.md             # the WorkspaceProvider port and what each impl guarantees
│   └── cache.md                # the WorkspaceCache port and the schema it projects
├── architecture.md      # Phase 2 output
├── design.md            # Phase 2 output
└── tasks.md             # Phase 3 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
protocol/src/
└── wire.rs                              # EXTENDED — workspace method params and results

client/core/src/
├── domain/
│   ├── workspace.rs                     # NEW — WorkspaceId, Workspace, RelPath, FsEntry,
│   │                                    #       FsMeta, ByteRange, FileChunk, Sha256
│   └── cache.rs                         # NEW — CacheEntry, Validity, RetentionWindow
├── application/
│   ├── ports/
│   │   ├── workspace_provider.rs        # NEW — the §6.1 trait (dyn, async_trait)
│   │   ├── workspace_cache.rs           # NEW — the projection as a capability
│   │   ├── bulk_transfer.rs             # NEW — fetch beside the channel (A-BULK)
│   │   └── clock.rs                     # NEW — so retention is testable without waiting
│   └── use_cases/
│       ├── cached_workspace.rs          # NEW — implements WorkspaceProvider over cache+engine
│       ├── register_workspace.rs        # NEW — mint, attach, delete (A-WORKSPACE)
│       ├── maintain_cache.rs            # NEW — migrate then evict, before anything opens
│       └── search_paths.rs              # NEW — FTS query, offline path search
├── adapters/
│   ├── inbound/
│   │   └── tauri_commands.rs            # EXTENDED — workspace commands for the webview
│   └── outbound/
│       ├── sqlite/
│       │   ├── mod.rs                   # NEW — SqliteWorkspaceCache
│       │   ├── schema.rs                # NEW — v1 DDL, the §5.2 canonical schema
│       │   └── migrate.rs               # NEW — user_version ladder, discard-and-rebuild
│       ├── remote_workspace.rs          # NEW — WorkspaceProvider over RequestTransport
│       ├── bulk/mod.rs                  # NEW — ssh invocation, ControlMaster=no
│       └── system_clock.rs              # NEW — the Clock port's real implementation
└── composition.rs                       # EXTENDED — maintenance runs before any open

client/core/tests/                       # NEW — the Rust suites, per quickstart.md
├── common/{fake_workspace,fake_cache,fake_clock}.rs
├── provider_contract.rs                 # every provider implementation runs this suite
├── cache_contract.rs                    # run against both SQLite and the in-memory fake
├── workspace_tree.rs                    # US1
├── cache_validity.rs                    # US2 — fail-first
├── cache_verification.rs                # US2 — the verifying window
├── workspace_registry.rs                # US3
├── cache_maintenance.rs                 # US4 — retention and migration
├── cache_offline.rs                     # US5
├── fts_sync.rs                          # US5 — the trigger gap
└── workspace_real_sshd.rs               # opt-in, real sshd

engine/tests/                            # NEW
├── path_containment.rs                  # fail-first, against a real tree
├── read_directory.rs
├── read_file.rs
└── register.rs

engine/src/
├── main.rs                              # REDUCED — composition root and stdio loop only
├── adapters/
│   ├── inbound/rpc.rs                   # NEW — method dispatch, was a match in main
│   └── outbound/std_fs.rs               # NEW — FileSystem over std::fs
├── application/
│   ├── ports/{file_system.rs,roots.rs}  # NEW — the two capabilities the engine needs
│   └── use_cases/workspace.rs           # NEW — read_directory, stat, read_file
├── domain/path.rs                       # NEW — canonicalise and assert containment (§4.7)
├── session.rs                           # EXISTING — untouched
└── handshake.rs                         # EXISTING — capability advertisement extended

client/ui/lib/
├── statusbar/presentation.ts            # EXTENDED — verifying and maintaining states
└── workspace/                           # NEW — tree store and the two indicators

tests/e2e/
├── workspace-tree.spec.ts               # NEW — US1
├── cache-verification.spec.ts           # NEW — US2 (FR-021b, SC-004b)
└── cache-maintenance.spec.ts            # NEW — US4 (FR-018a, SC-013a)
```

**Structure Decision**: Both binaries keep the hexagonal layout — the client extends the one F000
and F001 established, and the engine acquires it. The `protocol` crate gains the workspace method
shapes so the client and the engine share one definition of them, which is the same reasoning that
put framing there in F002 and is not a layering violation for the same reason: a protocol is by
definition shared with the process at the other end of the wire.

The one deliberate absence: **no `LocalWorkspaceProvider`.** §6.4 specifies one and the feature map
lists "local and remote implementations" under F003, but the local provider exists to serve Local
Mode, which is F015. Building it here would produce an adapter with no consumer, no acceptance
scenario and no way to fail. The port is what F015 needs from this feature, and the port is
delivered. An in-memory fake implementation ships for tests, which is what proves the trait is
genuinely implementable twice.

## Complexity Tracking

> No Constitution Check violations. This table is intentionally empty.
