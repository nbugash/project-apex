# Tasks: Offline Editing

**Input**: Design documents from `/specs/010-offline-editing/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md,
architecture.md, design.md

## Format: `[ID] [P?] [Story] Description`

**Tests are not optional here.** Constitution Principle VII makes them a MUST: "the acceptance
scenarios in the feature's own specification are the source; each one MUST have a corresponding
automated test." spec.md has 29 scenarios, and every one is claimed by a task below.

## Path Conventions

Paths are repository-relative. `engine/` is the remote daemon, `client/core/` the Rust client,
`client/ui/` the webview, `tests/e2e/` the WebdriverIO suites. The layout is design.md's *Module &
File Layout* and must match it; where they disagree, one of the two documents is wrong.

---

**Every new file must appear in a list somewhere.** In this codebase that is
`generate_handler!` for a Tauri command, the enclosing `mod.rs` for a Rust module,
`wdio.conf.ts`'s spec arrays for an end-to-end spec, and `Cargo.toml` for a dependency. Analyze
runs 10 and 11 each found one of these missing, and F011 shipped four instances of the same shape
— an artifact that exists with nothing pointing at it. The tasks below name the registration
explicitly wherever they create something.

## Phase 1: Setup (Shared Infrastructure)

- [ ] T001 Add `diffy = "0.4"` to `client/core/Cargo.toml` with a comment recording why 0.4 rather than 0.5: 0.5 requires rustc 1.85 and this workspace's MSRV is 1.75, which `cargo add` reports and a reader would otherwise rediscover

---

- [ ] T001a In `tests/e2e/wdio.conf.ts`, add `'./offline-*.spec.ts'` to the **live** `specs` list and to the ordinary suite's `exclude` list, beside the git and editor entries. Without it these three specs match no live pattern, so they would run in the ordinary suite — where there is no engine to disconnect from, which is the one thing they all require — and never run in the live one. F011 had to do exactly this for `git-*.spec.ts`; the registration is invisible until the suite quietly runs the wrong set

## Phase 2: Foundational (Blocking Prerequisites)

**Everything here blocks more than one user story. Nothing here delivers user value on its own.**

The `recentlyChanged` wire types were here in the first draft and are now in Phase 7, where they
belong: they serve US5 alone, so keeping them in Foundational misstated the dependency graph and
would have orphaned them if US5 were deferred — which this file offers as the escape hatch.

- [ ] T005 In `client/core/src/adapters/outbound/sqlite/schema.rs`, add `CURRENT_VERSION = 4` and `V4` creating `pending_edits` per data-model.md: keyed `(workspace_id, relative_path)`, **`base_blob` and `base_sha256` both nullable and set together**, `mergeable` not null, foreign key cascading from `workspaces`. The base content is stored rather than referenced (FR-011b) because `file_contents` is evictable and is overwritten by any refetch, so a referenced base would be gone on exactly the path the merge exists for
- [ ] T006 In `client/core/src/adapters/outbound/sqlite/migrate.rs`, add `4 => schema::V4`
- [ ] T007 `client/core/tests/migrate_v4.rs`: migrating a **populated** version-3 database preserves every workspace, file, cached blob and git row. Nothing has ever written a `pending_edits` row at migration time, so a migration that dropped the database and recreated it would satisfy every check about the new table perfectly — what must survive is everything else, which is the assertion F011's V3 test learned to make
- [ ] T008 In `client/core/src/application/ports/workspace_cache.rs`, add `PendingEdit` — carrying local content, base content and base hash — and the three operations from design.md: `retain_edit`, `pending_edits`, `forget_pending`
- [ ] T009 Implement those three in `client/core/src/adapters/outbound/sqlite/mod.rs`, re-validating `relative_path` on read as well as on write (Principle VI) and dropping a row whose path does not validate rather than repairing it
- [ ] T010 [P] Implement them on the two test doubles: `client/core/tests/common/fake_cache.rs` and the `RecordingCache` in `client/core/src/application/use_cases/search_paths.rs`
- [ ] T011 `client/core/tests/pending_store.rs`: a pending edit survives a **reopened** store; a row exists for a path with no `files` row and no `file_contents` row; `forget_pending` removes exactly one workspace's one path; and the row holds content and a base and **nothing else**, which is FR-034's bound against an outbox for arbitrary operations. Asserted on a reopened store because an in-memory map beside the database satisfies every assertion that keeps one handle open

---

## Phase 3: User Story 1 — Know I am offline, and keep reading (Priority: P1) 🎯 MVP

**Goal**: The interface says it is offline, every previously opened file still reads, and nothing
that needs the engine pretends otherwise.

**Independent test**: Open several files, cut the connection, and confirm the state is shown, every
opened file reads, and no action hangs waiting for a reply that is not coming.

### Tests for User Story 1

- [ ] T012 [P] [US1] `tests/e2e/offline-state.spec.ts`: US1 scenario 1, FR-002 and FR-003 — the status bar shows a distinct offline state **within 2 seconds** of the connection dropping, and no tab closes or resets. The elapsed time is printed, so SC-001 is a number rather than a verdict
- [ ] T013 [P] [US1] In `tests/e2e/offline-state.spec.ts`, US1 scenario 2, FR-005 and SC-012: a cached file opens offline with **zero** requests issued. The count is the assertion; F011's listing counter exists because `toBeGreaterThanOrEqual(0)` passes for an implementation that does nothing
- [ ] T014 [P] [US1] In `tests/e2e/offline-state.spec.ts`, US1 scenario 3: a folder never listed is marked unavailable rather than shown empty — and the distinction is asserted on the DOM, because "shown empty" and "marked unavailable" look identical to a test that only counts rows
- [ ] T015 [P] [US1] In `tests/e2e/offline-state.spec.ts`, US1 scenarios 4, 5 and 6 and FR-004, FR-007, FR-008, FR-009: path search returns cached results without claiming completeness, and **every row §11.3's table marks unavailable offline** states that it requires the engine — content search, code intelligence, terminals and tasks — with git state the last known state, marked, not cleared. Driven from the table rather than from a list in the test, so a row added there fails this test instead of being silently uncovered
- [ ] T016 [P] [US1] `client/core/tests/offline_budget.rs`: SC-007 and SC-008 **asserted and printed** — the test fails above 200 ms for a cached open and above 1 second for path search over 50,000 cached paths, and prints both measured values. Printing alone is not what Principle V requires: it demands a measurement that *fails* when the budget is exceeded. Measured through the real provider and the real store, not a double returning a clone, because F011's first budget test printed `0 us` and could not tell a fast client from one that was not running
- [ ] T016a [P] [US1] In `client/core/tests/offline_budget.rs`, `contracts/offline-commands.md` `offline_status` guarantees 1 and 2: the reported state follows the **published connection state** and nothing else — a request that fails while the connection state still says connected does **not** flip it to offline — and reading it issues **zero** requests. The first half is the one an implementation gets wrong by treating a timeout as evidence, which is how an offline indicator starts lying during a slow request
- [ ] T017 [P] [US1] `tests/unit/offline-presentation.test.ts`: the offline state and the "requires the engine" wording map to design-system tokens and are distinguishable **without colour**, compared by luminance following `tests/e2e/rail-greyscale.spec.ts`

### Implementation for User Story 1

- [ ] T018 [US1] `client/ui/lib/offline/state.svelte.ts`: `OfflineStore`, taking `connected` from the connection state F001 already publishes and **never re-detecting it** (FR-001). A second detector of a state already known is the defect A-RECONNECT records
- [ ] T018a [US1] In `client/core/src/adapters/inbound/tauri_commands.rs` and `client/core/src/composition.rs`, add `connection: Arc<ObserveConnection>` to `WorkspaceAccess` and wire it. Needed because `CachedWorkspace::connected()` is private and is not on the `WorkspaceProvider` trait, so no command can reach it — without this, `offline_status` and the offline routing of `file_write` have nowhere to get the answer from. Rejected alternative: routing inside `CachedWorkspace::write_file`, which already checks it, but that widens the caching layer's job and hides the offline branch from where the `HeldLocally` outcome is produced
- [ ] T019 [US1] In `client/core/src/adapters/inbound/tauri_commands.rs`, `offline_status` per `contracts/offline-commands.md`: the workspace resolved in the core, never accepted from the view, and no engine contact on this path
- [ ] T019a [US1] In `client/core/src/lib.rs`, add `cmd::offline_status` to `generate_handler!`. `lib.rs` already carries the warning from the feature that learned this: "Registered here or the webview's `invoke` resolves to nothing and every open fails as an unknown command." F006 hit it with four commands and F011 with four more; the failure is a runtime unknown-command error, so it surfaces only when the interface calls it
- [ ] T020 [US1] In `client/ui/lib/statusbar/StatusBar.svelte`, the offline indicator built from design-system tokens only, carrying an icon and a word so the state is not held in colour alone (FR-002, FR-004)
- [ ] T021 [US1] In `client/ui/lib/workspace/FileTree.svelte`, mark a folder that was never listed as unavailable rather than rendering it empty (FR-006)
- [ ] T022 [US1] In `client/ui/lib/shell/Window.svelte`, subscribe `OfflineStore` once for the window, on the same terms as the git store and the file-event router: a background tab must not be the reason a state stops being reported

**Checkpoint**: offline is visible and the cache is readable. Nothing is editable yet.

---

## Phase 4: User Story 2 — Keep editing, and lose nothing (Priority: P1)

**Goal**: The editor stays writable offline and saved work survives anything short of losing the
disk.

**Independent test**: Offline, edit and save several files, quit the application, relaunch it still
offline, and confirm every saved edit is present.

### Tests for User Story 2

- [ ] T023 [P] [US2] `client/core/tests/retain_edit.rs`: US2 scenarios 1 and 2 and FR-011 — a save while disconnected produces a pending edit carrying the base hash, and the outcome the caller receives says the work is held locally rather than written
- [ ] T024 [P] [US2] In `client/core/tests/retain_edit.rs`, US2 scenario 5 and FR-014: a file created offline is retained with a **NULL** base, because there is nothing for it to differ from. Asserted on the stored row, since a base of `""` would read as a hash everywhere downstream
- [ ] T025 [P] [US2] In `client/core/tests/retain_edit.rs`, US2 scenario 6 and FR-016: where the store refuses, the error reaches the caller while the work is still in the buffer, rather than being logged and swallowed
- [ ] T025a [P] [US2] In `client/core/tests/retain_edit.rs`, FR-017: retaining an offline edit leaves cached content and its hashes **untouched**. Compared by fingerprint before and after, because §5.3 says validity is a hash comparison and nothing else, and a retainer that quietly marked content stale would force a refetch of every file being worked on
- [ ] T025b [P] [US2] In `client/core/tests/retain_edit.rs`, FR-011b and FR-011c and SC-002a: the pending edit carries the base **content**, and saving the same file three times offline leaves that base unchanged while the content advances. Asserted on the stored base rather than on the merge's result, because a base re-derived from the newer local content makes the merge compare local against local — it returns a clean merge that is simply wrong, and no assertion about success would notice
- [ ] T026 [P] [US2] In `client/core/tests/retain_edit.rs`, FR-011a: an **unsaved** buffer is not retained. This is the half the reviewer decided during clarify, and an implementation that persisted keystrokes would pass every other test here
- [ ] T026a [P] [US2] In `client/core/tests/retain_edit.rs`, FR-016a and edge case 2: a file **deleted locally while offline** produces no pending work and is not propagated on reconnection. Asserted because the alternative — inferring a deletion from an absence — cannot tell "deleted" from "never cached"
- [ ] T027 [US2] In `tests/e2e/offline-state.spec.ts`, US2 scenarios 3 and 4 and FR-012 and SC-002: at least 50 saved edits across at least 10 files survive a genuine **quit and relaunch** while still offline, and reopening a file shows the developer's content rather than the host's last. The application is quit, not reloaded — a reload would leave the store open and prove nothing about persistence
- [ ] T028 [US2] In `tests/e2e/offline-state.spec.ts`, FR-015: a file whose work is held locally is distinguishable in the interface from one whose work is on the host

### Implementation for User Story 2

- [ ] T029 [US2] `client/core/src/application/use_cases/retain_edit.rs` (declared in `use_cases/mod.rs`): `RetainEdit::save` per design.md — on a path that already has offline work it replaces the **content** and leaves the **base untouched** (FR-011c), and it returns the store's error rather than absorbing it
- [ ] T030 [US2] In `client/core/src/adapters/inbound/tauri_commands.rs`, route `file_write` to `RetainEdit` when `access.connection` reports anything other than connected (T018a added the field), returning a `HeldLocally` outcome in the **success** channel beside F006's existing four. F006 established why: these are outcomes the interface must branch on, and splitting them across `Ok` and `Err` pushes the caller back to inspecting an error to find out which it was
- [ ] T031 [US2] In `client/ui/lib/editor/EditorPanel.svelte`, keep the editor writable while offline — no `readOnly` — including for a file the client cannot merge (FR-010, FR-017a) — and show that the file's work is held locally
- [ ] T032 [US2] In `client/ui/lib/editor/buffers.svelte.ts`, serve a reopened buffer from the pending edit rather than from the cached content when one exists (FR-013)

**Checkpoint**: work made offline cannot be lost. Nothing reconciles yet; that is US3.

---

## Phase 5: User Story 3 — Reconnect without merging by hand (Priority: P2)

**Goal**: On reconnection, work lands with no interaction wherever it can.

**Independent test**: Make offline edits, change unrelated files and unrelated regions on the host,
reconnect, and confirm everything lands with no prompt.

### Tests for User Story 3

- [ ] T033 [P] [US3] `client/core/tests/merge_agreement.rs`: SC-006b — for at least 20 file pairs, the client's decision to merge or prompt matches `git merge-file` in **100%** of cases. The corpus MUST contain a pair whose changes are on **neighbouring lines**, because that is the case a zero-context merge gets wrong and every other case would not; research.md's measurement is the starting corpus
- [ ] T034 [P] [US3] In `client/core/tests/merge_agreement.rs`, FR-020a asserted as behaviour: adjacent-line changes conflict, two-lines-apart changes merge. Named separately from the corpus above so a failure says which property broke
- [ ] T035 [P] [US3] `client/core/tests/reconcile.rs`: US3 scenario 1 and FR-018, FR-019 — where the host still matches the base, the local content is written with no interaction, and the pending row is gone afterwards
- [ ] T036 [P] [US3] In `client/core/tests/reconcile.rs`, US3 scenario 2 and FR-020: a host change to a different region of the same file combines, both changes present afterwards, no prompt
- [ ] T037 [P] [US3] In `client/core/tests/reconcile.rs`, US3 scenario 3 and FR-023: reconciliation is per file — a file that reached the host is no longer pending and one that did not still is, in the same run
- [ ] T038 [US3] In `client/core/tests/reconcile.rs`, US3 scenario 4 and FR-028: a connection lost **between two files** leaves no file partly written and every unreconciled edit retained. The interruption must land between files, which needs more than one file with work — an interruption inside a single file's write proves a different thing
- [ ] T038a [P] [US3] In `client/core/tests/reconcile.rs`, FR-011b and edge case 8: a file whose **cached content has been evicted** still merges, because the base travels with the pending edit. The eviction must be real — the `file_contents` row genuinely removed — or the test passes against an implementation that reads the base from the cache
- [ ] T039 [P] [US3] In `client/core/tests/reconcile.rs`, FR-022: a row is deleted **only** where the host confirmed a write. Driven by making the write fail after the merge succeeded, which is the ordering a careless implementation gets wrong
- [ ] T040 [P] [US3] `client/core/tests/reconcile_budget.rs`: SC-011 **asserted and printed** — the test fails above 10 seconds for reconciling 100 files with pending edits, and prints the measured value (Principle V)
- [ ] T040a [P] [US3] In `client/core/tests/reconcile.rs`, edge cases 3, 5 and 16: reconciliation with the **workspace root gone** reports per file rather than emptying the tree; an application **quit during reconciliation** resumes on the next reconnection with every unwritten edit intact; and a reconnection whose **protocol version is incompatible** (§3.8) does not reconcile at all, because a workspace that cannot be used cannot be reconciled
- [ ] T041 [US3] `tests/e2e/offline-reconcile.spec.ts`: US3 scenarios 1, 2 and 5 against a real engine — a clean reconnection costs **zero** developer interactions (SC-004), a non-overlapping host change still costs zero (SC-005), and the developer is shown what happened per file rather than inferring it

### Implementation for User Story 3

- [ ] T042 [P] [US3] `client/core/src/application/ports/text_merge.rs`: the `TextMerge` port and `MergeOutcome`, declared in `application/ports/mod.rs`, with `Conflict` carrying nothing — a partially merged file is not a thing this feature may produce
- [ ] T042a [P] [US3] `client/core/tests/merge_confinement.rs`: assert `diffy` is named in exactly one source file, `adapters/outbound/text_merge.rs`. The same rule and reason as `engine/tests/inotify_confinement.rs`: if the merge library may be named anywhere then anywhere may decide what a conflict is, and the conflict boundary is what SC-006b pins down. Strip comments before checking, because F011's first separation guard fired on its own rationale. **Both halves, as the shipped guard has them**: assert the adapter *does* name `diffy`, and assert no other file does. Without the first, deleting or renaming the adapter makes the guard pass by finding nothing to complain about — `inotify_confinement.rs:70` calls that "the vacuous form of a structural guard", and it is the form F011 shipped three times before mutation caught them. With both halves the test fails before T043 exists, which is the ordinary red of a test written first, not a guard that cannot fail
- [ ] T043 [US3] `client/core/src/adapters/outbound/text_merge.rs`: `DiffyMerge`, declared in `adapters/outbound/mod.rs`, the only file naming `diffy` (T042a enforces it)
- [ ] T044 [US3] `client/core/src/application/use_cases/reconcile.rs` (declared in `use_cases/mod.rs`): `Reconcile::run` per design.md's sequence — read the host, compare against the base, write or merge or conflict, and delete the row only inside the transaction that commits the write. The base is the write protocol's existing `baseSha256` and no new protocol field is introduced (FR-027)
- [ ] T045 [US3] In `client/core/src/application/use_cases/reconcile.rs`, `ReconcileReport` and `Outcome`, returned rather than raised: a reconciliation that returned `Err` would lose the per-file detail FR-024 requires
- [ ] T046 [US3] In `client/core/src/composition.rs`, wire `Reconcile` to run once per transition into `Connected`, from the connection state already published (A-RECONNECT). Not a timer, not the first successful request
- [ ] T047 [US3] In `client/core/src/adapters/inbound/tauri_commands.rs` and `client/ui/lib/offline/state.svelte.ts`, surface the reconciliation report to the interface (FR-024)

**Checkpoint**: offline work reaches the host wherever it can do so unambiguously.

---

## Phase 6: User Story 4 — See a real conflict, and decide it myself (Priority: P2)

**Goal**: Where changes genuinely collide, the developer decides and the client writes nothing
until they do.

**Independent test**: Edit the same lines offline and on the host, reconnect, and confirm the
developer is asked and neither version is written until they answer.

### Tests for User Story 4

- [ ] T048 [P] [US4] In `client/core/tests/reconcile.rs`, US4 scenarios 1 and 2 and FR-021, FR-033 and SC-006: overlapping changes prompt, **nothing** is written to the host, and the offline work is still retained while the developer has not answered
- [ ] T049 [P] [US4] In `client/core/tests/reconcile.rs`, US4 scenario 6 and FR-025a and SC-006a: a file the client cannot merge prompts **even when the host did not change it**. This is the half an implementation would quietly drop, and the one the reviewer chose against the recommendation
- [ ] T050 [P] [US4] In `client/core/tests/reconcile.rs`, US4 scenario 5 and FR-026: a file deleted on the host while edited offline prompts rather than letting the deletion or the edit win
- [ ] T051 [P] [US4] In `client/core/tests/reconcile.rs`, US4 scenario 3: resolving writes the resolution and clears the pending row, in one transaction — neither without the other
- [ ] T052 [P] [US4] In `client/core/tests/reconcile.rs`, `contracts/offline-commands.md` guarantee 2 on `conflict_resolve`: a write refused as stale becomes a **new conflict against the newer remote**, not an error the developer has to interpret
- [ ] T052a [P] [US4] In `client/core/tests/merge_agreement.rs`, US4 scenario 7: two offline edits in different regions of one file, with a host change close to one of them, prompt **for that region on version-control merge terms** rather than for the file as a whole. The fixture must put the third change near one edit and far from the other, or the test cannot tell per-region from per-file
- [ ] T052b [P] [US4] In `client/core/tests/reconcile.rs`, `contracts/offline-commands.md` `conflicts_list` guarantees 2 and 3: the remote side is **read when the list is built**, so changing the host between two listings changes what the second shows; and `base` is empty **only** for a file created offline, not for one whose cached content was evicted. The first guards against resolving against a remote that went stale while the developer was deciding — a silent wrong answer, not a failure
- [ ] T053 [US4] `tests/e2e/offline-conflict.spec.ts`: US4 scenario 4 and FR-025 and SC-003 — a conflict left unresolved survives going offline again, and across the full cycle of edit, disconnect, relaunch, reconnect **zero** work is lost for any outcome including conflict and is presented on the next reconnection. The second disconnection is real and the conflict genuinely unresolved, not resolved and re-created
- [ ] T054 [P] [US4] `tests/unit/offline-presentation.test.ts`: the conflict panel's three sides are labelled and distinguishable without colour, and a conflict with no cached base still renders

### Implementation for User Story 4

- [ ] T055 [US4] In `client/core/src/adapters/inbound/tauri_commands.rs`, `conflicts_list` per contract — the remote side read when the list is built, never stored, so it cannot go stale while the developer decides
- [ ] T056 [US4] In `client/core/src/adapters/inbound/tauri_commands.rs`, `conflict_resolve` — write and forget in one transaction, and a stale refusal becomes a fresh conflict
- [ ] T056a [US4] In `client/core/src/lib.rs`, add `cmd::conflicts_list` and `cmd::conflict_resolve` to `generate_handler!`, for the reason T019a records
- [ ] T057 [P] [US4] `client/ui/lib/offline/conflicts.svelte.ts`: `ConflictStore`, refreshed rather than cached, for the same staleness reason
- [ ] T058 [US4] `client/ui/lib/offline/ConflictPanel.svelte`: the three sides and the choice, from design-system tokens only. **Recorded as a deviation** in spec.md and plan.md: the prototype has no conflict screen, the same shape as F011's branch indicator
- [ ] T059 [US4] In `client/ui/lib/shell/Window.svelte`, surface outstanding conflicts where the developer will see them without hunting

**Checkpoint**: nothing can be silently lost or silently chosen.

---

## Phase 7: User Story 5 — Have what I need before I lose the connection (Priority: P3)

**Goal**: Going offline is not a lottery about which files happen to be cached.

**Independent test**: Work online without opening the manifests or recently changed files, go
offline, and confirm they are readable.

### Tests for User Story 5

- [ ] T060 [P] [US5] `engine/tests/git_recent.rs`: parse `git log --name-only` output into a deduplicated path set — a file changed in five of twenty commits appears once (`contracts/recently-changed.md` guarantee 4)
- [ ] T061 [P] [US5] In `engine/tests/git_recent.rs`, guarantee 3 and FR-003a: a workspace on a **subdirectory** receives its own paths re-rooted and nothing from elsewhere in the repository, on the same terms `git/getStatus` gives
- [ ] T062 [P] [US5] In `engine/tests/git_recent.rs`, guarantee 5: a directory that is not a repository, and a host without git, each answer successfully with an **empty list**
- [ ] T063 [P] [US5] In `engine/tests/git_recent.rs`, guarantee 2: `commits` defaults to 20 and is capped at 100; a caller asking for more gets 100
- [ ] T063a [P] [US5] In `engine/tests/git_recent.rs`, `contracts/recently-changed.md` guarantee 1: the result carries **paths only** — no file content, no commit identities, no authors, no dates. Asserted on the serialised **payload**, not on the parser, because a parser that discards the extra fields and a result that carries them look identical from the parser's side. F011's equivalent guarantee was untrue until exactly this test was written
- [ ] T063b [P] [US5] In `engine/tests/git_recent.rs`, guarantee 6: a result that would exceed §4.1's frame cap is **truncated rather than refused**, and no cursor is offered. Truncation is not an error: prefetch is speculative and a partial answer is a partial prefetch. Driven with a repository whose recent commits genuinely touch more paths than one frame holds, or the branch never executes
- [ ] T064 [P] [US5] `client/core/tests/prefetch.rs`: US5 scenarios 1 and 3 and FR-029, FR-032 and SC-010 — manifests and recent-commit files are cached without being asked for, asserted against prefetch's **own report** of what it fetched rather than against a wall-clock wait, and a workspace with no repository still caches manifests and does not fail
- [ ] T065 [US5] In `client/core/tests/prefetch.rs`, US5 scenario 5 and FR-029a and SC-010a: with the cache at its budget, prefetch **stops**, evicts nothing, and reports stopping rather than failing. The cache must genuinely be at its budget and the opened file genuinely least-recently-used, or the check passes for an implementation that evicts freely
- [ ] T066 [P] [US5] In `client/core/tests/prefetch.rs`, US5 scenario 4 and FR-031: an interrupted prefetch leaves no half-written cache entry
- [ ] T067 [US5] `client/core/tests/prefetch_budget.rs`: US5 scenario 2, FR-030 and SC-009 **asserted and printed** — the test fails when interactive latency during prefetch exceeds its idle latency by more than 10%, and prints both (Principle V)

### Implementation for User Story 5

- [ ] T067a [P] [US5] In `protocol/src/wire.rs`, add `RecentlyChangedParams` and `RecentlyChangedResult` per `contracts/recently-changed.md`, snake_case on the wire (A-WIRECASE), with `commits` optional
- [ ] T067b [P] [US5] `protocol/tests/recent_wire.rs`: hand-written JSON round-trips for both types, asserting the wire spelling rather than trusting the derive
- [ ] T068 [P] [US5] In `engine/src/application/ports/git.rs`, add `recently_changed` to the `Git` port
- [ ] T069 [US5] In `engine/src/adapters/outbound/git_cli.rs`, implement it with `git log --name-only --pretty=format: -n <commits> -- .`, honouring the prefix re-rooting FR-003a established and the config pinning already there. **All three flags matter and were verified against git 2.43**: without `--pretty=format:` the parser meets commit headers; without `-- .` the walk covers the whole repository's history, and while the prefix strip would still drop the outsiders, the work is wasted on exactly the monorepo the scoping exists for; paths come back repository-root-relative, so the same prefix machinery as `status` applies unchanged
- [ ] T070 [US5] In `engine/src/adapters/inbound/rpc.rs`, dispatch `git/recentlyChanged`, refusing an unregistered workspace with `-32001` and a non-positive `commits` with `-32602`
- [ ] T071 [P] [US5] In `client/core/src/application/ports/git_provider.rs` and `client/core/src/adapters/outbound/remote_git.rs`, add `recently_changed` through the transport
- [ ] T072 [US5] `client/core/src/application/use_cases/prefetch.rs` (declared in `use_cases/mod.rs`): `Prefetch::run` — manifests first, then recent-commit paths, checking the budget before each fetch and stopping rather than evicting
- [ ] T073 [US5] In `client/core/src/composition.rs`, run prefetch at background priority (§4.6) so it cannot delay interactive traffic (FR-030)

**Checkpoint**: all five subfeatures delivered.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [ ] T074 [P] Update `docs/app-shell.md` with the client side: why a pending edit is its own table and outlives its cache entry, why the row is the durable fact and every outcome a statement about one attempt, and why the remote side of a conflict is never stored
- [ ] T075 [P] Update `docs/engine.md` with `git/recentlyChanged`: paths only, capped, not paged, and why a cursor would be the opposite of a bounded prefetch
- [ ] T076 Per `quickstart.md` §5, run **every** mutation it lists and record each outcome in §8. Each must fail **with the assertion expected** rather than with a compile error. §5 states which matter most and why; that ranking is not repeated here, because it changed twice while this line said otherwise — the count and the ranking live in §5 alone
- [ ] T077 Per `quickstart.md` §4, audit **every** negative check it lists and confirm for each that the condition which lets it fail is actually present — a real binary rather than a text file named `.bin`, a genuinely untouched host side, a cache genuinely at its budget, an interruption genuinely between two files
- [ ] T078 In `quickstart.md` §6, record the ten measurements from §3 in the *Validation record*, each number beside its bound. A gate that says only PASS tells nobody how much headroom is left
- [ ] T079 Confirm every one of spec.md's 29 acceptance scenarios is named by at least one test, and every FR and SC is cited by name in a test file. F011 shipped with three requirements tested but uncited, which made a coverage audit read them as gaps
- [ ] T080 Verify `make gate` is green, then mark F012's five subfeatures in `specs/features-map.md` and run `feature_map.py verify`

---

## Dependencies & Execution Order

### Phase Dependencies

```text
Phase 1 Setup ──▶ Phase 2 Foundational ──┬──▶ Phase 3 US1 ──▶ Phase 4 US2 ──▶ Phase 5 US3 ──▶ Phase 6 US4
                                          └──▶ Phase 7 US5
                                                                                    Phase 8 Polish ◀── all
```

### User Story Dependencies

- **US1** depends only on Foundational. It is the MVP.
- **US2** depends on US1 in practice, not in principle: retaining an edit needs to know the client
  is offline, and US1 is where that becomes known.
- **US3** depends on US2 — there is nothing to reconcile until something is retained.
- **US4** depends on US3: a conflict is an outcome of reconciliation.
- **US5** is **independent of US1 through US4** and could be built first or last. It is last only
  because it is P3; it is the one story that degrades rather than breaks the feature by its absence.

### Within Each User Story

Tests before the implementation they cover, in every phase. The order is the project's practice
and Principle VII's requirement, and it is what made F011's defects visible before they shipped.

### Parallel Opportunities

**What `[P]` means here.** The tasks are independent *as work* — no task depends on another's
result — which is what makes them safe to plan in any order. It does **not** license two writers
in one file at the same time. Several tasks deliberately add cases to one test file, and the
convention is: **one writer per file at a time, whatever the marker says.**

This is worth stating rather than fixing by deleting markers. Thirty-eight tasks write a file
another task also writes, mostly because a test file gathers a story's cases — `reconcile.rs` has
thirteen, `retain_edit.rs` seven, `git_recent.rs` six. Stripping `[P]` from all of them would lose
the information that the cases are independent, which is what matters when deciding what to leave
until later.

**Genuinely parallel — different files, no shared writer:**

- Protocol, schema and port work in Phase 2: different crates.
- US5's engine tests against any client story's work: different crates.
- The two documentation tasks in Phase 8: different documents.

**Files with more than one writer**, which must be taken one task at a time:

| File | Tasks |
|---|---|
| `client/core/src/adapters/inbound/tauri_commands.rs` | six, across US1, US2 and US4 |
| `client/core/tests/reconcile.rs` | thirteen, across US3 and US4 |
| `client/core/tests/retain_edit.rs` | seven, in US2 |
| `engine/tests/git_recent.rs` | six, in US5 |
| `tests/e2e/offline-state.spec.ts` | six, across US1 and US2 |
| `client/core/tests/merge_agreement.rs` | three, across US3 and US4 |
| `client/core/tests/prefetch.rs` | three, in US5 |
| `client/core/tests/offline_budget.rs`, `tests/unit/offline-presentation.test.ts` | two each |

Described by file rather than by task id on purpose: an enumerated id list goes stale on the next
insertion, which is exactly how the previous version of this section came to name four
`tauri_commands.rs` tasks when there were six.

## Parallel Example: User Story 1

```text
# All six tests for User Story 1 together:
T012  tests/e2e/offline-state.spec.ts        (scenario 1, SC-001)
T013  tests/e2e/offline-state.spec.ts        (scenario 2, SC-012)   [same file: write together, one task at a time]
T016  client/core/tests/offline_budget.rs    (SC-007, SC-008)
T017  tests/unit/offline-presentation.test.ts

# Then the implementation, where only T018 and T019 are independent of each other:
T018  client/ui/lib/offline/state.svelte.ts
T019  client/core/src/adapters/inbound/tauri_commands.rs
```

## Implementation Strategy

### MVP First (User Story 1 Only)

Phases 1 through 3 deliver a client that says it is offline and stays readable. That is shippable
on its own and is the half of A-B3's original read-only behaviour worth keeping.

### Incremental Delivery

Each phase's checkpoint is a state the feature could stop at without being incoherent:

1. **After US1** — offline is visible and the cache is readable. No editing.
2. **After US2** — work made offline cannot be lost, but it does not leave the machine.
3. **After US3** — work reaches the host wherever it can do so unambiguously.
4. **After US4** — nothing can be silently lost or silently chosen. This is the point A-OFFLINE
   was reversed to reach.
5. **After US5** — offline coverage is deliberate rather than accidental.

**The scope note from spec.md applies here.** This is one cycle covering what A-OFFLINE priced at
roughly three features, which the reviewer decided deliberately. The mitigation planning can offer
is that US3 and US4 — the merge and the conflict interface, the part that can destroy a
colleague's work — are their own phases with their own tests, their own port, their own adapter
and their own confinement guard, so they can be reviewed without reading the rest.
