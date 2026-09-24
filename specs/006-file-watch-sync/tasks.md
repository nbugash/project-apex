---
description: "Task list for F004 file-watch-sync"
---

# Tasks: File Watch Sync

**Input**: Design documents from `/specs/006-file-watch-sync/`

**Prerequisites**: [plan.md](./plan.md), [research.md](./research.md), [data-model.md](./data-model.md),
[contracts/](./contracts/), [architecture.md](./architecture.md), [design.md](./design.md),
[quickstart.md](./quickstart.md)

**Tests are required, not optional.** Constitution Principle VII binds every feature to unit,
integration and end-to-end levels, FR-028 requires every behaviour to be verifiable with no remote
host and no network, and A-TEST makes the standard binding. Test tasks below are deliverables.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: may run in parallel — different file, no dependency on an incomplete task
- **[US1]**–**[US4]**: the user story this task serves

## Path Conventions

Rust workspace: `protocol/`, `engine/`, `client/core/`. Webview: `client/ui/src/lib/`.
End-to-end specs: `tests/e2e/`. Engine MSRV is **1.75** and the engine adds **no async runtime**.

---

## Phase 1: Setup

**Purpose**: the one dependency, the test double that cannot yet carry a server-initiated frame,
and the one design answer this feature is owed.

- [ ] T001 Add `inotify = "0.11"` to `[dependencies]` in `engine/Cargo.toml`, with a comment stating it is Linux-only and confined to one adapter. Do **not** add an async runtime: the existing comment in that file records that the engine stays synchronous because it is transferred on every first connect
- [ ] T002 Add a `notify=<ms>` directive to `client/core/tests/mock_daemon/main.rs` that writes a **caller-supplied opaque frame** (read from `APEX_MOCK_FRAME`) after the given delay. The mock must not know what it is sending — `the_mock_implements_no_engine_method` at line 251 fails the build if any §4.8 method name appears in that directory, and the method name must therefore live in the calling test's string
- [ ] T003 [P] Document the `notify` directive in the table in `client/core/tests/mock_daemon/README.md`, stating why it carries an opaque frame rather than a named method
- [ ] T004 Record the Principle I answer for the changed-on-host tab marker in `specs/006-file-watch-sync/spec.md` under a new `## Design deviations` heading. The prototype binds the 6px tab dot to `t.dirty → var(--color-accent)` meaning *unsaved local changes*; FR-023a needs a distinct marker. **This task gates T085 and T086 only.** Every other task proceeds without it

---

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: the wire vocabulary, the ports, the pure coalescer, the writer seam and the schema
migration. Every user story depends on all of it.

**⚠️ No user story phase may begin until this phase is complete.**

### Protocol vocabulary

- [ ] T005 Add `WatchParams`, `WatchResult`, `Refusal` and `RefusalReason` to `protocol/src/wire.rs` per [data-model.md](./data-model.md) *Wire types*, snake_case on the wire with no `rename_all`, matching the convention F002 established
- [ ] T006 Add `FileEventParams` carrying `events[]`, and `FileEventKind` with `created`/`modified`/`deleted`/`renamed`, to `protocol/src/wire.rs`. `created` and `modified` carry `type`, `size`, `modified`; only `renamed` sets `to_path` (depends on T005, same file)
- [ ] T007 Add `InvalidateAllParams` to `protocol/src/wire.rs` and extend `wire::codes` with the constants it is missing — `-32000`, `-32004`, `-32005`, `-32006`, `-32008` — so no integer is ever written inline, which that module's own doc comment already requires (depends on T006, same file)
- [ ] T008 [P] Round-trip serialisation tests for every new wire type in `protocol/tests/watch_wire.rs`, asserting the snake_case field names explicitly rather than round-tripping into Rust and back — a symmetric bug survives a round trip

### Engine domain and ports

- [ ] T009 [P] Implement `WatchId`, `Watch`, `WatchSet`, `RawEvent`, `RawKind` (including `Overflow`) and `FileEvent` in `engine/src/domain/watch.rs` per [data-model.md](./data-model.md). `WatchSet` insert and remove are idempotent, and its unit tests assert that adding the same directory twice yields one watch
- [ ] T010 [P] Define the `FileWatcher` port in `engine/src/application/ports/file_watcher.rs` — `watch`, `unwatch`, `poll`, `held` — with `WatchError` carrying `CapacityExhausted`, `NotADirectory` and `Gone`, per [contracts/watcher-port.md](./contracts/watcher-port.md). `Send` but not `Sync` (W8), and the reason recorded in a doc comment
- [ ] T011 [P] Define the `Clock` port in `engine/src/application/ports/clock.rs` returning monotonic `Millis`. Milliseconds rather than `Instant` so a fake clock is a number, which is what makes the volume tests arithmetic instead of sleeping
- [ ] T012 [P] Register the new modules in `engine/src/application/ports/mod.rs`
- [ ] T013 [P] Implement `ExclusionSet` in `engine/src/application/exclusions.rs`: `.gitignore` files plus the fixed built-in set from §10.3 (`.git/`, `node_modules/`, `target/`, `dist/`, `build/`, `.venv/`, `__pycache__/`). No per-workspace configuration (FR-009)
- [ ] T014 [P] Unit tests for `ExclusionSet` in `engine/tests/exclusions.rs` covering a nested `.gitignore`, a negation (`!`), and a path that is excluded by the built-in set but absent from every `.gitignore`

### The coalescer — pure, and where the volume requirements are actually tested

- [ ] T015 Implement the per-path window in `engine/src/application/coalescer.rs`: `accept`, `drain_due` returning `Emission`, and `next_deadline`. 100 ms trailing edge (A-COALESCE). No filesystem, no threads, no `serde` in scope — it is fed events and told the time
- [ ] T016 Unit tests in `engine/src/application/coalescer.rs` for FR-012 and SC-007: a thousand `accept` calls for one path across one simulated second yields **at most 10** events, and — the assertion that matters — **at least 1**. An upper bound alone passes when the window is widened to infinity and the developer waits forever (depends on T015, same file)
- [ ] T017 Implement rename pairing in `engine/src/application/coalescer.rs`: `IN_MOVED_FROM` and `IN_MOVED_TO` sharing a cookie inside the window become one `renamed` event naming both paths; an unpaired half at flush becomes `deleted` or `created` respectively (FR-011, depends on T016, same file)
- [ ] T018 Unit tests in `engine/src/application/coalescer.rs` for rename pairing, including both unpaired directions — a file moved out of the workspace is a deletion here, and one moved in is a creation (depends on T017, same file)
- [ ] T019 Implement the trailing edge explicitly in `engine/src/application/coalescer.rs` and test it: the **last** write in a burst is the one whose event is emitted, not the first (contracts/file-events.md guarantee 5, depends on T018, same file)

### The writer seam — new, and the thing FR-016 measures

- [ ] T020 Implement `FrameWriter` in `engine/src/adapters/outbound/frame_writer.rs` as the sole owner of stdout, taken per frame and released before the next. This seam does not exist today: `rpc.rs` returns frames and the session loop writes them, because until F004 there was never a second writer
- [ ] T021 Generalise `encode_notification` in `engine/src/adapters/inbound/rpc.rs` from `params: &RestartNotice` to `params: &T where T: Serialize`, so it can carry a file event
- [ ] T022 Route the existing session-loop writes through `FrameWriter` in `engine/src/main.rs` and `engine/src/session.rs`, so there is exactly one writer before a second one is introduced (depends on T020)
- [ ] T023 [P] Integration test in `engine/tests/frame_writer.rs` asserting two concurrent writers never interleave a frame, by writing from two threads and parsing the result stream

### Schema version 2

- [ ] T024 Add `V2` to `client/core/src/adapters/outbound/sqlite/schema.rs` — `files.stale` and `file_contents.unproven`, both `INTEGER NOT NULL DEFAULT 0`, plus `DROP TRIGGER files_fts_update` and its recreation with `AFTER UPDATE OF relative_path, name`. Bump `CURRENT_VERSION` to 2. `V2` contains only the delta, never V1 repeated
- [ ] T025 Add the `2 => schema::V2` arm to `client/core/src/adapters/outbound/sqlite/migrate.rs`, in the same transaction that sets `user_version`, so a failed migration leaves a v1 database rather than a half-migrated one (depends on T024)
- [ ] T026 [P] Migration test in `client/core/tests/migrate_v2.rs`: build a v1 database, migrate, assert both columns exist, assert `user_version` is 2, and assert the narrowed trigger no longer fires on an update that touches only `stale` — the point of narrowing it
- [ ] T027 [P] Regression test in `client/core/tests/migrate_v2_idempotent.rs` asserting a second migration run is a no-op and a v2 database is left untouched

### Port signature changes

- [ ] T028 Replace the F004 `Unsupported` stub in `client/core/src/application/ports/workspace_provider.rs` with `watch(&self, ws, paths: &[RelPath])` and `unwatch(&self, ws, paths: &[RelPath])`, both returning `ProviderResult<WatchOutcome>`. `paths` carries **what the client cares about** — folder paths and file paths — not the directories the engine will watch
- [ ] T029 Add `mark_stale`, `mark_unproven` and `rename_subtree` to `client/core/src/application/ports/workspace_cache.rs`. `rename_subtree` returns the number of rows rewritten, because that count is what a test asserts the separator boundary against
- [ ] T030 [P] Extend the in-memory fake in `client/core/tests/support/fake_cache.rs` with the three new methods, so use-case tests stay free of SQLite

**Checkpoint**: `cargo build --workspace` succeeds, `cargo clippy --workspace --all-targets -- -D warnings` is clean, and no user story has begun.

---

## Phase 3: User Story 1 — See a colleague's change without asking for it (P1) 🎯 MVP

**Goal**: a change made on the host to a folder the developer has expanded appears in the tree
without them asking.

**Independent test**: expand a folder, create and delete a file in it on the host, and watch the
tree update, with no remote host and no network.

### Tests for User Story 1

- [ ] T031 [P] [US1] Contract test in `engine/tests/watch_methods.rs` for `workspace/watch`: params and result shapes, idempotency, and that watching an already-watched path changes nothing
- [ ] T032 [P] [US1] Contract test in `engine/tests/file_events.rs` for `workspace/onFileEvent`: the array shape, the four event kinds, `to_path` present only on `renamed`, and `type`/`size`/`modified` present on `created` and `modified`
- [ ] T033 [P] [US1] Integration test in `engine/tests/watch_flow.rs` driving a `FakeWatcher` and a fake clock end to end: a created file yields one event carrying the metadata needed to place it in a tree
- [ ] T034 [P] [US1] Use-case test in `client/core/tests/apply_file_event.rs`: a `created` event with metadata produces a `files` row without any further request, which is what FR-020 requires and what the `NOT NULL` columns make impossible without the metadata

### Implementation for User Story 1

- [ ] T035 [P] [US1] Implement `FakeWatcher` in `engine/tests/support/fake_watcher.rs` — feed events, exhaust capacity on demand, report `held()` — with no filesystem
- [ ] T036 [P] [US1] Implement `FakeClock` in `engine/tests/support/fake_clock.rs` returning a settable monotonic `Millis`
- [ ] T037 [US1] Implement `SetWatchedPaths` in `engine/src/application/use_cases/watch.rs`: resolve and contain every path through `ResolvedPath`, skip excluded ones, derive directories from file paths, and return `WatchOutcome { watching, refused }`. Capacity exhaustion is returned, never raised (W1, FR-005a)
- [ ] T038 [US1] Implement the `InotifyWatcher` adapter in `engine/src/adapters/outbound/inotify_watcher.rs`. **The only file in the repository that may name `inotify`**; map `ENOSPC` to `WatchError::CapacityExhausted` and `IN_Q_OVERFLOW` to `RawKind::Overflow`
- [ ] T039 [US1] Implement the watch thread in `engine/src/adapters/outbound/watch_thread.rs`: poll for exactly `next_deadline()`, drain into the coalescer, emit due batches through `FrameWriter`. One `std::thread`, no runtime (depends on T020, T038)
- [ ] T040 [US1] Dispatch `workspace/watch` and `workspace/unwatch` in `engine/src/adapters/inbound/rpc.rs`, threading the per-workspace watched set and the `FileWatcher` through `dispatch`. Remove the now-stale comment `let _ = (roots, fs); // threaded through for the workspace methods that land in Phase 3` (depends on T021)
- [ ] T041 [US1] Store the resolved `ExclusionSet` on the registered workspace in `engine/src/session.rs` at `workspace/register`, so the future indexer reads the same set rather than computing a second one (A-IGNORE, FR-007)
- [ ] T042 [US1] Implement `ApplyFileEvent` in `client/core/src/application/use_cases/apply_file_event.rs`: re-validate containment independently of the engine, then correct the projection. Never marks content valid, never fetches (FR-014, FR-019, FR-020)
- [ ] T043 [US1] Implement the notification inbound adapter in `client/core/src/adapters/inbound/file_event_notification.rs`, translating the wire payload into use-case input and carrying no business rule
- [ ] T044 [US1] Implement `watch`/`unwatch` in `client/core/src/adapters/outbound/remote_workspace.rs` over the transport
- [ ] T045 [P] [US1] Return `Unsupported` from `watch`/`unwatch` in `client/core/src/adapters/outbound/local_workspace.rs`, with the A-WATCHLOCAL reasoning in a doc comment
- [ ] T046 [US1] Wire the watcher, clock and writer in the engine composition root in `engine/src/lib.rs`, with no global singleton
- [ ] T047 [US1] Drive watch requests from folder expansion in `client/ui/src/lib/tree/FileTree.svelte`, debounced, sending folder paths on expand and releasing on collapse
- [ ] T048 [US1] Apply created, modified and deleted events to the rendered tree in place in `client/ui/src/lib/tree/treeStore.ts`, without collapsing or re-fetching the folder
- [ ] T049 [P] [US1] Unit test in `client/ui/tests/unit/tree-apply-event.test.ts` asserting an applied event does not reset expansion state

**Checkpoint**: US1 is independently testable — expand, change on the host, see the tree update.

---

## Phase 4: User Story 2 — Survive a branch switch without a flood (P1)

**Goal**: a change affecting thousands of paths arrives as one wholesale invalidation, and
interactive work continues throughout.

**Independent test**: with a folder expanded, apply ten thousand changes at once and confirm one
invalidation, a stale tree, no refetch, and interactions still inside budget.

### Tests for User Story 2

- [ ] T050 [P] [US2] Unit test in `engine/tests/bulk_threshold.rs`: 255 distinct paths in one second yield individual events; 256 yield exactly one `invalidateAll` and **zero** individual events for that window (FR-015, SC-004)
- [ ] T051 [P] [US2] Unit test in `engine/tests/overflow.rs`: a `RawKind::Overflow` yields `invalidateAll`, because events the kernel dropped are changes nobody would otherwise hear about (A-COALESCE, FR-005)
- [ ] T052 [P] [US2] Integration test in `client/core/tests/invalidate_all.rs`: a wholesale invalidation marks the tree stale and discards **zero** content blobs (FR-018, SC-006)
- [ ] T053 [P] [US2] Integration test in `client/core/tests/lazy_requery.rs`: after an invalidation, zero listing requests are issued until the developer navigates (FR-017, SC-012a)

### Implementation for User Story 2

- [ ] T054 [US2] Implement the bulk rule in `engine/src/application/coalescer.rs`: 256 distinct paths within a rolling 1-second window emits `Emission::InvalidateAll` and discards the individual events for that window (depends on T019, same file)
- [ ] T055 [US2] Map `RawKind::Overflow` to `Emission::InvalidateAll` in `engine/src/application/coalescer.rs` (depends on T054, same file)
- [ ] T056 [US2] Emit `workspace/invalidateAll` from `engine/src/adapters/outbound/watch_thread.rs` when the coalescer returns it (depends on T039)
- [ ] T057 [US2] Implement `mark_stale` over `files.stale` in `client/core/src/adapters/outbound/sqlite/mod.rs`, marking a whole workspace in one statement
- [ ] T058 [US2] Handle `invalidateAll` in `client/core/src/application/use_cases/apply_file_event.rs`: mark stale, discard nothing, fetch nothing (depends on T042)
- [ ] T059 [US2] Re-read a stale region only when the developer navigates into it, in `client/core/src/application/use_cases/cached_workspace.rs`
- [ ] T060 [P] [US2] Render staleness by dimming in `client/ui/src/lib/tree/FileTree.svelte` following the prototype's own treatment — "Dimmed rows are stale … They are never waited on" — using existing tokens and no raw values (Principle I)

**Checkpoint**: a branch switch produces one invalidation and no flood.

---

## Phase 5: User Story 3 — Know that the file you are reading has moved on (P1)

**Goal**: a file with an open tab is reported as changed; a file with no open tab does not
interrupt anybody.

**Independent test**: open a file, change it on the host, and confirm the developer is told
without the file being altered underneath them and without focus moving.

### Tests for User Story 3

- [ ] T061 [P] [US3] Integration test in `client/core/tests/unproven.rs`: an event naming a cached file sets `unproven`, discards zero blobs and triggers zero fetches (FR-019a, SC-006a)
- [ ] T062 [P] [US3] Integration test in `client/core/tests/unproven_offline.rs`: an unproven blob is still served while disconnected, presented as possibly stale (FR-019b, SC-006b)
- [ ] T063 [P] [US3] Test in `client/core/tests/rename_subtree.rs` asserting the separator boundary: renaming `src` rewrites `src` and everything under `src/`, and leaves `src-generated` **untouched**. Assert on the returned row count, not only on spot checks
- [ ] T064 [P] [US3] Test in `client/core/tests/rename_subtree_atomic.rs`: a rewrite that fails partway rolls back entirely, leaving a consistent stale projection rather than a half-renamed one
- [ ] T065 [P] [US3] Test in `client/core/tests/rename_own_row.rs`: the renamed directory's **own** row gets its caller-derived parent, not the `substr` arithmetic that works for its descendants — one formula would set the directory's parent to itself

### Implementation for User Story 3

- [ ] T066 [US3] Implement `mark_unproven` over `file_contents.unproven` in `client/core/src/adapters/outbound/sqlite/mod.rs`, leaving `Validity` untouched — unproven is a flag beside validity, never a validity state (A-UNPROVEN, depends on T057)
- [ ] T067 [US3] Clear `unproven` when a hash comparison runs, in `client/core/src/application/use_cases/cached_workspace.rs`, so the existing check is the only thing that decides (depends on T059)
- [ ] T068 [US3] Implement `rename_subtree` in `client/core/src/adapters/outbound/sqlite/mod.rs` with the exact-row plus `LIKE 'from/%'` match expressed as an explicit range comparison, in one transaction, with the `CASE` that gives the renamed row its own parent (depends on T066)
- [ ] T069 [US3] Derive watched paths from open tabs in `client/ui/src/lib/tabs/tabStore.ts`, sending **file** paths so the engine keeps their folder watched after a collapse (FR-003c)
- [ ] T070 [US3] Apply rename events to the tree in `client/ui/src/lib/tree/treeStore.ts`, moving the entry rather than removing and re-adding it so cached content survives (FR-021, depends on T048)
- [ ] T071 [P] [US3] Unit test in `client/ui/tests/unit/tab-watch-paths.test.ts`: opening a file whose folder is collapsed still contributes a watched path, and closing its last tab removes it

**Checkpoint**: an open file that changes on the host is reported; one with no tab is not.

---

## Phase 6: User Story 4 — Trust that watching stopped when it should (P2)

**Goal**: watches are released when they should be, re-established when they should be, and their
absence is always stated.

**Independent test**: close a workspace, drop the connection, restart the engine and exhaust
capacity, and confirm resources are returned and the developer is told in every case.

### Tests for User Story 4

- [ ] T072 [P] [US4] Integration test in `engine/tests/watch_release.rs`: closing a workspace returns `held()` to its pre-open level with zero watches left (SC-009)
- [ ] T073 [P] [US4] Integration test in `engine/tests/watch_proportional.rs`: a workspace of a hundred thousand files with ten folders expanded holds watches for those ten, their ancestors and any open tabs outside them, and no more (SC-009a)
- [ ] T074 [P] [US4] Integration test in `engine/tests/watch_collapse.rs`: collapsing a folder releases its watch, and collapsing one that still holds an open tab releases **zero** watches that tab depends on (SC-009b)
- [ ] T075 [P] [US4] Integration test in `engine/tests/watch_exhausted.rs` against a `FakeWatcher` with capacity exhausted: the workspace still opens and browses, and every unwatchable path appears in `refused[]` (FR-005a, SC-009c)
- [ ] T076 [P] [US4] Integration test in `client/core/tests/reconnect_watches.rs`: every folder still expanded and every file still open at reconnection is being watched again afterwards, including a tab whose folder is collapsed (FR-026b, SC-012b)
- [ ] T077 [P] [US4] Integration test in `client/core/tests/reconnect_stale.rs`: reconnection marks the tree stale, discards zero blobs, and issues zero listings until the developer navigates (FR-026, FR-026a, SC-012a)
- [ ] T078 [P] [US4] Integration test in `client/core/tests/watch_unavailable.rs`: when watching is unavailable the developer is told in 100% of cases and zero silent failures occur (FR-005, FR-025, SC-011)
- [ ] T079 [P] [US4] Integration test in `client/core/tests/event_outside_root.rs`: an event naming a path outside the workspace root is refused by the client and writes nothing, independently of the engine (FR-014, SC-010)

### Implementation for User Story 4

- [ ] T080 [US4] Implement `ReleaseWatches` in `engine/src/application/use_cases/watch.rs`: release on collapse, on the last tab closing, on workspace close, on connection drop and on engine exit (FR-004, depends on T037)
- [ ] T081 [US4] Return `-32009` when the workspace root has gone, distinct from `-32001`, in `engine/src/adapters/inbound/rpc.rs`; refuse a deleted watched path per-path with `not_found` rather than failing the whole call, which would leave everything unwatched on re-establishment (depends on T040)
- [ ] T082 [US4] Re-establish the full watched set with one `watch` call on reconnection in `client/core/src/application/use_cases/observe_connection.rs`, carrying expanded folders and open files together (FR-026b)
- [ ] T083 [US4] Mark the tree stale in its entirety on reconnection in `client/core/src/application/use_cases/observe_connection.rs`, re-reading nothing until the developer navigates (FR-026, depends on T082, same file)
- [ ] T084 [US4] Surface watch refusals to the developer in `client/core/src/domain/connection.rs` as a state distinct from disconnection — exhausted capacity happens while perfectly connected, so `ConnectionState` alone is the wrong home
- [ ] T085 [US4] Render the "changes are not being reported" indication in `client/ui/src/lib/shell/StatusBar.svelte` using existing design tokens (FR-025, FR-027; **depends on T004**)
- [ ] T086 [US4] Render the changed-on-host tab marker in `client/ui/src/lib/tabs/TabStrip.svelte` per the answer recorded in T004, keeping it distinct from the dirty dot bound to `t.dirty` (FR-023a; **depends on T004**)

**Checkpoint**: watching stops, resumes and reports its own absence correctly.

---

## Phase 7: Polish and cross-cutting concerns

- [ ] T087 [P] Measure reflection latency in `tests/perf/watch-reflection.mjs` — p99 over at least 100 samples at the interface boundary, harness delay excluded, **printing the measured value** rather than only comparing it (A-NFR, SC-001). Measure one interval across a locally spawned engine; summing an engine-side p99 and a client-side p99 gives a p98 bound, not a p99, and if that composition is used it must be reported as p98
- [ ] T088 [P] Measure watch establishment against the §1.4 interaction budget in `tests/perf/watch-establish.mjs`, printing the measured value (Principle V, A-NFR)
- [ ] T089 [P] Measure event delivery against interactive traffic in `tests/perf/watch-interference.mjs`: with ten thousand changes in flight, interactive actions still meet §1.4 (FR-016, SC-005)
- [ ] T090 [P] End-to-end spec in `tests/e2e/file-watch.spec.ts`: expand a folder, change a file on the host, see the tree update, and confirm an unfocused tab is marked with **zero** focus changes (SC-001a)
- [ ] T091 [P] End-to-end spec in `tests/e2e/file-watch-stale.spec.ts`: a wholesale invalidation dims the tree and navigation re-reads it
- [ ] T092 [P] Greyscale and keyboard-reachability assertions for both new visual states in `tests/e2e/file-watch-a11y.spec.ts`, because `lint:ds` can see neither
- [ ] T093 [P] Run the mutation checks from [quickstart.md](./quickstart.md) and record the outcome: widen the coalescing window, remove the separator boundary from the subtree rename, and make an event mark content valid. Each must fail a named test. **The window mutation is the one that matters** — FR-012's assertion is an upper bound, so widening the window makes the event count fall and a suite with no lower bound still passes while the developer waits
- [ ] T094 [P] Update `docs/engine.md` with the watcher, the coalescer and the writer seam
- [ ] T095 [P] Update `docs/workspace-cache.md` with schema version 2, the unproven flag and the subtree rename rule
- [ ] T096 Verify `make gate` is green, then mark F004 complete in `specs/features-map.md`

---

## Dependencies

```text
Setup (T001-T004)
   └─> Foundational (T005-T030)   ← blocks every story
          ├─> US1 (T031-T049)  🎯 MVP
          ├─> US2 (T050-T060)   depends on US1's coalescer and apply path
          ├─> US3 (T061-T071)   depends on US1's apply path
          └─> US4 (T072-T086)   depends on US1's watch establishment
                 └─> Polish (T087-T096)
```

T004 gates only T085 and T086. Every other task is unblocked by it.

## Parallel opportunities

- **Foundational**: T009–T014 are six different files and run together; T008, T023, T026, T027, T030 likewise
- **US1**: all four test tasks T031–T034 in parallel; then T035, T036, T045, T049
- **US2**: T050–T053 in parallel
- **US3**: T061–T065 in parallel — five different test files
- **US4**: T072–T079 in parallel — eight different test files, the largest parallel block in the feature
- **Polish**: T087–T095 in parallel

The coalescer tasks (T015–T019, T054, T055) are deliberately **not** parallel: they are one file, and
marking them `[P]` would be the same false claim that shipped seventeen times across F000, F002 and
F018.

## Implementation strategy

**MVP is US1 alone.** It delivers the feature's whole point — a change you did not make appears
without you asking — and it is independently shippable. US2 protects it from floods, US3 extends it
from folders to open files, US4 makes its absence honest.

Stop after US1 and the feature is useful. Stop after US2 and it is safe. US3 and US4 are what make
it trustworthy.
