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
`wdio.conf.ts`'s spec arrays for an end-to-end spec, `Cargo.toml` for a dependency, `migrate.rs`'s match for a schema version, and — found in analyze run 23 — the webview's `WriteOutcome` union **and** `describeOutcome`'s switch for a new save outcome, since a variant the interface cannot name is a variant the developer is told the wrong thing about. Analyze
runs 10 and 11 each found one of these missing, and F011 shipped four instances of the same shape
— an artifact that exists with nothing pointing at it. The tasks below name the registration
explicitly wherever they create something.

**One of those lists is ordered.** `wdio.conf.ts`'s live `specs` array must keep
`terminal-live.spec.ts` last: the config records, in thirteen lines and from measurement, that
every spec which ran after it stalled in its `before` hook for minutes while passing on its own.
Appending is the natural reading of "add it to the list" and is the wrong one, and the symptom is
a hang rather than a failure. T001a states the position; T079a checks the interaction did not
gain a second instance, since the offline specs restart the engine much as that spec does.

**No Rust source file names a URL.** There are currently zero across `engine/src`, `client/core/src` and `protocol/src`, and `make no-network` enforces it on machines without unprivileged user namespaces by failing when a source file matches `https?://`. This feature is the one most likely to break it — a new crate and a new adapter invite a doc comment linking to `diffy`'s documentation or `git merge-file`'s manual page. Cite `research.md` by name instead. The failure is worse than a red gate: it depends on whether the machine has user namespaces, so the same commit is green for one developer and red for another.

**Do not restate a count another document derives.** Say "every mutation §5 lists", not "the eight
mutation checks". Analyze runs 12 and 13 between them found five stale counts — the mutation total,
the mutation ranking, the requirement and criterion totals in plan.md, the measurement total, and
the number of tasks sharing a file — every one of them correct when written and wrong after the
next remediation added something. A count is a derived fact, and Principle II applies to a derived
fact exactly as it applies to a stored one. The scenario count is the one exception, restated
deliberately because the reviewer's scope decision fixes it rather than remediation.

## Phase 1: Setup (Shared Infrastructure)

- [X] T001 Add `diffy = "0.4"` to `client/core/Cargo.toml` with a comment recording why 0.4 rather than 0.5: 0.5 requires rustc 1.85 and this workspace's MSRV is 1.75, which `cargo add` reports and a reader would otherwise rediscover

---

- [X] T001a In `tests/e2e/wdio.conf.ts`, add `'./offline-*.spec.ts'` to the **live** `specs` list and to the ordinary suite's `exclude` list. Without it these three specs match no live pattern, so they would run in the ordinary suite — where there is no engine to disconnect from, which is the one thing they all require — and never run in the live one. F011 had to do exactly this for `git-*.spec.ts`; the registration is invisible until the suite quietly runs the wrong set. **Position it before `'./terminal-live.spec.ts'`, never after.** That array's order is load-bearing and the config says so in thirteen lines above it: `terminal-live` starts a real login shell, and every spec that ran after it stalled in its `before` hook for minutes while passing on its own. Appending is the natural reading of "add to the list" and is the wrong one — the failure is a hang, not an assertion, and it arrives minutes later in a suite that was green the day before

## Phase 2: Foundational (Blocking Prerequisites)

**Everything here blocks more than one user story. Nothing here delivers user value on its own.**

The `recentlyChanged` wire types were here in the first draft and are now in Phase 7, where they
belong: they serve US5 alone, so keeping them in Foundational misstated the dependency graph and
would have orphaned them if US5 were deferred — which this file offers as the escape hatch.

- [X] T005 In `client/core/src/adapters/outbound/sqlite/schema.rs`, add `CURRENT_VERSION = 4` and `V4` creating `pending_edits` per data-model.md: keyed `(workspace_id, relative_path)`, **`base_blob` and `base_sha256` both nullable and set together**, `mergeable` not null, foreign key cascading from `workspaces`. The base content is stored rather than referenced (FR-011b) because `file_contents` is evictable and is overwritten by any refetch, so a referenced base would be gone on exactly the path the merge exists for
- [X] T006 In `client/core/src/adapters/outbound/sqlite/migrate.rs`, add `4 => schema::V4`
- [X] T007 `client/core/tests/migrate_v4.rs`: migrating a **populated** version-3 database preserves every workspace, file, cached blob and git row. Nothing has ever written a `pending_edits` row at migration time, so a migration that dropped the database and recreated it would satisfy every check about the new table perfectly — what must survive is everything else, which is the assertion F011's V3 test learned to make
- [X] T008 In `client/core/src/application/ports/workspace_cache.rs`, add `PendingEdit` — carrying local content, base content and base hash — and the three operations from design.md: `retain_edit`, `pending_edits`, `forget_pending`
- [X] T009 Implement those three in `client/core/src/adapters/outbound/sqlite/mod.rs`, re-validating `relative_path` on read as well as on write (Principle VI) and dropping a row whose path does not validate rather than repairing it
- [X] T010 [P] Implement them on the two test doubles: `client/core/tests/common/fake_cache.rs` and the `RecordingCache` in `client/core/src/application/use_cases/search_paths.rs`
- [X] T011 `client/core/tests/pending_store.rs`: a pending edit survives a **reopened** store; a row exists for a path with no `files` row and no `file_contents` row; `forget_pending` removes exactly one workspace's one path; and the row holds content and a base and **nothing else**, which is FR-034's bound against an outbox for arbitrary operations. Asserted on a reopened store because an in-memory map beside the database satisfies every assertion that keeps one handle open

---

## Phase 3: User Story 1 — Know I am offline, and keep reading (Priority: P1) — MVP

**Goal**: The interface says it is offline, every previously opened file still reads, and nothing
that needs the engine pretends otherwise.

**Independent test**: Open several files, cut the connection, and confirm the state is shown, every
opened file reads, and no action hangs waiting for a reply that is not coming.

### Tests for User Story 1

- [X] T011a `client/core/tests/pending_store.rs`: a row with **one base column set and not the other** is treated as unmergeable and logged once, never merged (data-model.md's validation rules, design.md's error table). Write the row through SQL rather than through the port, since the port is what is supposed to make the state impossible — a test that can only build the row the legal way asserts nothing about the illegal one. The file must **prompt**: merging content against a base that is absent rather than empty is the wrong-clean-merge shape mutation 10 exists for, arriving from a malformed row instead of a re-derived one
- [X] T012 [US1] `tests/e2e/offline-state.spec.ts`: US1 scenario 1, FR-002 and FR-003 — the status bar shows a distinct offline state **within 2 seconds** of the connection dropping, and no tab closes or resets. The elapsed time is printed, so SC-001 is a number rather than a verdict
- [X] T013 [US1] In `tests/e2e/offline-state.spec.ts`, US1 scenario 2, FR-005 and SC-012: a cached file opens offline with **zero** requests issued. The count is the assertion; F011's listing counter exists because `toBeGreaterThanOrEqual(0)` passes for an implementation that does nothing
- [X] T014 [US1] In `tests/e2e/offline-state.spec.ts`, US1 scenario 3: a folder never listed is marked unavailable rather than shown empty — and the distinction is asserted on the DOM, because "shown empty" and "marked unavailable" look identical to a test that only counts rows
- [X] T015 [US1] In `tests/e2e/offline-state.spec.ts`, US1 scenarios 4, 5 and 6 and FR-004, FR-007, FR-008, FR-009: path search returns cached results without claiming completeness, and **every row §11.3's table marks unavailable offline** states that it requires the engine — content search, code intelligence, terminals and tasks — with git state the last known state, marked, not cleared. Driven from the table rather than from a list in the test, so a row added there fails this test instead of being silently uncovered
- [X] T016 [US1] `client/core/tests/offline_budget.rs`: SC-007 and SC-008 **asserted and printed** — the test fails above 200 ms for a cached open and above 1 second for path search over 50,000 cached paths, and prints both measured values. Printing alone is not what Principle V requires: it demands a measurement that *fails* when the budget is exceeded. Measured through the real provider and the real store, not a double returning a clone, because F011's first budget test printed `0 us` and could not tell a fast client from one that was not running
- [X] T016a [US1] In `client/core/tests/offline_budget.rs`, `contracts/offline-commands.md` `offline_status` guarantees 1 and 2: the reported state follows the **published connection state** and nothing else — a request that fails while the connection state still says connected does **not** flip it to offline — and reading it issues **zero** requests. The first half is the one an implementation gets wrong by treating a timeout as evidence, which is how an offline indicator starts lying during a slow request
- [X] T016b [US1] In `client/core/tests/offline_budget.rs`, `contracts/offline-commands.md` `offline_status` guarantee 3: `pending` is every file with retained work **in the current workspace** and nothing from another. Register two workspaces, retain work in both, and assert each reports only its own — a single-workspace fixture passes whether or not the query is scoped, which is what makes the leak invisible. The table is keyed `(workspace_id, relative_path)` precisely so this can be scoped; a query that forgets the first half of the key still returns plausible rows
- [X] T017 [US1] `tests/unit/offline-presentation.test.ts`: the offline state and the "requires the engine" wording map to design-system tokens and are distinguishable **without colour**, compared by luminance following `tests/e2e/rail-greyscale.spec.ts`

### Implementation for User Story 1

- [X] T018 [US1] `client/ui/lib/offline/state.svelte.ts`: `OfflineStore`, taking `connected` from the connection state F001 already publishes and **never re-detecting it** (FR-001). A second detector of a state already known is the defect A-RECONNECT records
- [X] T018a [US1] **Corrected during implementation, and not as written.** The task said to add `connection: Arc<ObserveConnection>` to `WorkspaceAccess`, because "`CachedWorkspace::connected()` is private and is not on the `WorkspaceProvider` trait, so no command can reach it". The second half is false: `Shell` already holds `connection` (`tauri_commands.rs:20`), both states are managed (`lib.rs:121-122`), and `session_get` already takes two `State` parameters — so a command could always reach it. What settles the design is not reachability but ownership: the connection is **one fact for the application, not one per workspace**. §13.1 and spec.md's Assumptions both say offline is a state of the connection and not of the workspace, so a `connection` field on `WorkspaceAccess` would encode the opposite. `offline_status` therefore takes `State<'_, Shell>` alongside `State<'_, WorkspaceAccess>`, and no field was added. The alternative the task did reject — routing inside `CachedWorkspace::write_file` — is still rejected, for the reason it gives.
- [X] T019 [US1] In `client/core/src/adapters/inbound/tauri_commands.rs`, `offline_status` per `contracts/offline-commands.md`: the workspace resolved in the core, never accepted from the view, and no engine contact on this path
- [X] T019a [US1] In `client/core/src/lib.rs`, add `cmd::offline_status` to `generate_handler!`. `lib.rs` already carries the warning from the feature that learned this: "Registered here or the webview's `invoke` resolves to nothing and every open fails as an unknown command." F006 hit it with four commands and F011 with four more; the failure is a runtime unknown-command error, so it surfaces only when the interface calls it
- [X] T020 [US1] In `client/ui/lib/statusbar/StatusBar.svelte`, the offline indicator built from design-system tokens only, carrying an icon and a word so the state is not held in colour alone (FR-002, FR-004)
- [X] T021 [US1] In `client/ui/lib/workspace/FileTree.svelte`, mark a folder that was never listed as unavailable rather than rendering it empty (FR-006)
- [X] T021a [US1] `client/ui/lib/workspace/PathSearch.svelte`: a filter input, a results list, and the **"showing cached results"** caveat when `complete` is false, from design-system tokens only. **Added during implementation**, not in the original list: T015 was to test scenario 4, and writing it showed FR-007 names a surface -- "the developer runs a path search" -- where the client had none, and where F005's `SearchPaths` had no command, no registration and no caller. The reviewer chose to build it here rather than defer to F013 or narrow FR-007. No debounce: SC-008 measures a p99 near 3 ms over 50,000 paths, so a debounce would add latency to every search to save work that is not expensive. Recorded as the second Principle I deviation, in spec.md's `## Design deviations`
- [X] T021b [US1] In `client/core/src/adapters/inbound/tauri_commands.rs` and `client/core/src/lib.rs`, `workspace_search_paths` with a `complete` flag, registered. The flag is the requirement and not the paths: a bare list leaves every caller free to render a partial cache as the whole repository, which is what FR-007 forbids. `search_complete` is extracted and unit-tested, because a requirement whose only test needs a real engine goes unchecked on every ordinary run
- [X] T022 [US1] In `client/ui/lib/shell/Window.svelte`, subscribe `OfflineStore` once for the window, on the same terms as the git store and the file-event router: a background tab must not be the reason a state stops being reported

**Checkpoint**: offline is visible and the cache is readable. Nothing is editable yet.

---

## Phase 4: User Story 2 — Keep editing, and lose nothing (Priority: P1)

**Goal**: The editor stays writable offline and saved work survives anything short of losing the
disk.

**Independent test**: Offline, edit and save several files, quit the application, relaunch it still
offline, and confirm every saved edit is present.

### Tests for User Story 2

- [X] T023 [US2] `client/core/tests/retain_edit.rs`: US2 scenarios 1 and 2 and FR-011 — a save while disconnected produces a pending edit carrying the base hash, and the outcome the caller receives says the work is held locally rather than written
- [X] T024 [US2] In `client/core/tests/retain_edit.rs`, US2 scenario 5 and FR-014: a file created offline is retained with a **NULL** base, because there is nothing for it to differ from. Asserted on the stored row, since a base of `""` would read as a hash everywhere downstream
- [X] T025 [US2] In `client/core/tests/retain_edit.rs`, US2 scenario 6 and FR-016: where the store refuses, the error reaches the caller while the work is still in the buffer, rather than being logged and swallowed
- [X] T025a [US2] In `client/core/tests/retain_edit.rs`, FR-017: retaining an offline edit leaves cached content and its hashes **untouched**. Compared by fingerprint before and after, because §5.3 says validity is a hash comparison and nothing else, and a retainer that quietly marked content stale would force a refetch of every file being worked on
- [X] T025b [US2] In `client/core/tests/retain_edit.rs`, FR-011b and FR-011c and SC-002a: the pending edit carries the base **content**, and saving the same file three times offline leaves that base unchanged while the content advances. Asserted on the stored base rather than on the merge's result, because a base re-derived from the newer local content makes the merge compare local against local — it returns a clean merge that is simply wrong, and no assertion about success would notice
- [X] T026 [US2] **Moved during implementation, because it could not fail where it was placed.** FR-011a -- an unsaved buffer is not retained -- is not observable in `retain_edit.rs`: the core never sees a keystroke, so the only test that file could hold is "nothing happens when nothing is called", which passed against a use case stubbed to `Ok(())`. It was written, the mutation caught it, and it is gone rather than left looking like coverage. The property now lives in `tests/e2e/offline-state.spec.ts`, which types into a buffer without saving, relaunches, and asserts the text is absent -- the only place an implementation that persisted keystrokes would be caught
- [X] T026a [US2] In `client/core/tests/retain_edit.rs`, FR-016a and edge case EC-02: a file **deleted locally while offline** produces no pending work and is not propagated on reconnection. Asserted because the alternative — inferring a deletion from an absence — cannot tell "deleted" from "never cached"
- [X] T027 [US2] In `tests/e2e/offline-state.spec.ts`, US2 scenarios 3 and 4 and FR-012 and SC-002: at least 50 saved edits across at least 10 files survive a genuine **quit and relaunch** while still offline, and reopening a file shows the developer's content rather than the host's last. The application is quit, not reloaded — a reload would leave the store open and prove nothing about persistence
- [X] T028 [US2] In `tests/e2e/offline-state.spec.ts`, FR-015: a file whose work is held locally is distinguishable in the interface from one whose work is on the host

### Implementation for User Story 2

- [X] T029 [US2] `client/core/src/application/use_cases/retain_edit.rs` (declared in `use_cases/mod.rs`): `RetainEdit::save` per design.md — on a path that already has offline work it replaces the **content** and leaves the **base untouched** (FR-011c), and it returns the store's error rather than absorbing it
- [X] T030 [US2] In `client/core/src/adapters/inbound/tauri_commands.rs`, route `file_write` to `RetainEdit` when `access.connection` reports anything other than connected (T018a added the field), returning a `HeldLocally` outcome in the **success** channel beside F006's existing four. Construct `RetainEdit` from `access.cache`, which `WorkspaceAccess` already carries — `tauri_commands.rs:513` holds it so a command can write through the same port the application reads through, and the pending-edit operations are on that port. No new field, unlike T018a: `connection` had to be added because `CachedWorkspace::connected()` is private and absent from the trait, and stating which of the two cases applies is what stops the next command guessing. F006 established why: these are outcomes the interface must branch on, and splitting them across `Ok` and `Err` pushes the caller back to inspecting an error to find out which it was
- [X] T031 [US2] In `client/ui/lib/editor/EditorPanel.svelte`, keep the editor writable while offline — no `readOnly` — including for a file the client cannot merge (FR-010, FR-017a) — and show that the file's work is held locally
- [X] T031a [US2] In `client/ui/lib/editor/buffers.svelte.ts`, add `{ kind: 'heldLocally' }` to the `WriteOutcome` union and treat it as a **success** in `save`: `buffers.svelte.ts:315` currently sends every outcome that is not `'written'` to `b.failed(outcome)`, so a save that was retained would appear in the failure surface — the confusion §11.2 forbids in so many words, arriving from the direction nobody checked. The union's own comment says "Four variants, never collapsed"; it becomes five. The `if/else` compiles either way, which is why this is the half that needs a test rather than a compiler
- [X] T031b [US2] **Half done, and half withdrawn during implementation.** The `heldLocally` case was added to `describeOutcome` -- held on this machine, will reconcile, `tone: 'ok'`. The instruction to **correct the `unreachable` copy was wrong and is not done.** Its premise was that an offline save is now retained, so `unreachable` must mean the write could not even be held; but T030 routes a save to the retainer *before* `EditFile` is reached whenever the connection is anything but connected, and a retain that fails returns `Refused` with its reason. So `unreachable` never describes the offline case: it still means a request that went out believing the link was up and never landed, which is exactly what F006 made it mean. F006's own tests -- that this case names the link and promises the work is still here -- failed the rewrite and were right to. Rewritten copy reverted, and the reasoning left in `ending.ts` so the next reader does not redo it
- [X] T031c [US2] In `tests/unit/offline-presentation.test.ts`, FR-012 and FR-016: `describeOutcome` returns `tone: 'ok'` for `heldLocally` and never claims the work was not saved, and the `unreachable` label no longer promises that saving again will work. This asserts the **decision**, not the presentation: `data-tone` is set on the notice in `EditorPanel.svelte:338` and styled nowhere in the product, so every tone renders identically today. T031a's half — that `save` does not route a retained outcome through `b.failed()` — is what keeps the developer from seeing a success in the failure surface. Assert over **every** variant of the union rather than the new one, so adding a sixth without a label fails here as well as in the compiler
- [X] T032 [US2] In `client/ui/lib/editor/buffers.svelte.ts`, serve a reopened buffer from the pending edit rather than from the cached content when one exists (FR-013)

**Checkpoint**: work made offline cannot be lost. Nothing reconciles yet; that is US3.

---

## Phase 5: User Story 3 — Reconnect without merging by hand (Priority: P2)

**Goal**: On reconnection, work lands with no interaction wherever it can.

**Independent test**: Make offline edits, change unrelated files and unrelated regions on the host,
reconnect, and confirm everything lands with no prompt.

### Tests for User Story 3

- [X] T033 [US3] `client/core/tests/merge_agreement.rs`: SC-006b — for at least 20 file pairs, the client's decision to merge or prompt matches `git merge-file` in **100%** of cases. The corpus MUST contain a pair whose changes are on **neighbouring lines**, because that is the case a zero-context merge gets wrong and every other case would not; research.md's measurement is the starting corpus. **Where `git` is absent this test fails and does not skip**, unlike `bootstrap_real_sshd.rs` and the rest of the house pattern. A developer following that convention would add a `skip_reason()` here, and this is the one test where skipping is the defect: SC-006b is the whole justification for taking `diffy` rather than writing the merge, so a silent skip retires the evidence for the feature's central decision while the suite stays green. Fail with a message naming the missing dependency. The test needing `git` does not contradict research.md's reason for not shelling out to it — that is about the shipped client on a developer's machine, and this runs on a machine with the repository checked out. It also runs under `make no-network`'s `unshare -rn`, where a subprocess is fine and a socket would not be
- [X] T034 [US3] In `client/core/tests/merge_agreement.rs`, FR-020a asserted as behaviour: adjacent-line changes conflict, two-lines-apart changes merge. Named separately from the corpus above so a failure says which property broke
- [X] T035 [US3] `client/core/tests/reconcile.rs`: US3 scenario 1 and FR-018, FR-019 — where the host still matches the base, the local content is written with no interaction, and the pending row is gone afterwards. The outcome must be **`FastForwarded`**, not `Merged`: both succeed and both delete the row, so nothing else in this file distinguishes them, and FR-024 requires the report say per file what happened. Reporting a merge for a fast-forward sends a developer to review a combination that never occurred
- [X] T036 [US3] In `client/core/tests/reconcile.rs`, US3 scenario 2 and FR-020: a host change to a different region of the same file combines, both changes present afterwards, no prompt, and the outcome is **`Merged`** — the other half of T035's distinction, which is only meaningful if both cases assert it
- [X] T037 [US3] In `client/core/tests/reconcile.rs`, US3 scenario 3 and FR-023: reconciliation is per file — a file that reached the host is no longer pending and one that did not still is, in the same run
- [X] T038 [US3] In `client/core/tests/reconcile.rs`, US3 scenario 4 and FR-028: a connection lost **between two files** leaves no file partly written and every unreconciled edit retained, and the remaining files are reported **`NotAttempted`** rather than `Failed` (design.md's error table). `Failed` leaves them retained too, so an assertion about the store alone passes either way — and the two mean opposite things to the developer: `NotAttempted` is retried on the next reconnection, `Failed` is not. This is FR-020b's defect one row down the same table. The interruption must land between files, which needs more than one file with work — an interruption inside a single file's write proves a different thing
- [X] T038a [US3] In `client/core/tests/reconcile.rs`, FR-011b and edge case EC-08: a file whose **cached content has been evicted** still merges, because the base travels with the pending edit. The eviction must be real — the `file_contents` row genuinely removed — or the test passes against an implementation that reads the base from the cache
- [X] T038b [US3] In `client/core/tests/reconcile.rs`, FR-020b: when the host's content changes **between the read and the write** of one reconciliation, the engine's `-32004` refusal is reported as a **conflict** and not a failure, and the pending row survives. The double is told to accept the read and then refuse the write, because a test that changes the host before reconciliation starts exercises the ordinary overlap path instead and passes either way. `Failed` keeps the row too (FR-022 holds by construction), so the assertion must read the **outcome**, not the store — the defect is a developer handed an error they cannot act on where a conflict they can resolve was available
- [X] T039 [US3] In `client/core/tests/reconcile.rs`, FR-022: a row is deleted **only** where the host confirmed a write. Driven by making the write fail after the merge succeeded, which is the ordering a careless implementation gets wrong
- [X] T040 [P] [US3] `client/core/tests/reconcile_budget.rs`: SC-011 **asserted and printed** — the test fails above 10 seconds for reconciling 100 files with pending edits, and prints the measured value (Principle V)
- [X] T040a [US3] In `client/core/tests/reconcile.rs`, edge cases EC-03, EC-05 and EC-16: reconciliation with the **workspace root gone** reports per file rather than emptying the tree; an application **quit during reconciliation** resumes on the next reconnection with every unwritten edit intact; and a reconnection whose **protocol version is incompatible** (§3.8) does not reconcile at all, because a workspace that cannot be used cannot be reconciled
- [ ] T041 [US3] `tests/e2e/offline-reconcile.spec.ts`: US3 scenarios 1, 2 and 5 against a real engine — a clean reconnection costs **zero** developer interactions (SC-004), a non-overlapping host change still costs zero (SC-005), and the developer is shown what happened per file rather than inferring it

### Implementation for User Story 3

- [X] T042 [P] [US3] `client/core/src/application/ports/text_merge.rs`: the `TextMerge` port and `MergeOutcome`, declared in `application/ports/mod.rs`, with `Conflict` carrying nothing — a partially merged file is not a thing this feature may produce
- [X] T042a [P] [US3] `client/core/tests/merge_confinement.rs`: assert `diffy` is named in exactly one source file, `adapters/outbound/text_merge.rs`. The same rule and reason as `engine/tests/inotify_confinement.rs`: if the merge library may be named anywhere then anywhere may decide what a conflict is, and the conflict boundary is what SC-006b pins down. Strip comments before checking, because F011's first separation guard fired on its own rationale. **Both halves, as the shipped guard has them**: assert the adapter *does* name `diffy`, and assert no other file does. Without the first, deleting or renaming the adapter makes the guard pass by finding nothing to complain about — `inotify_confinement.rs:70` calls that "the vacuous form of a structural guard", and it is the form F011 shipped three times before mutation caught them. With both halves the test fails before T043 exists, which is the ordinary red of a test written first, not a guard that cannot fail
- [X] T043 [US3] `client/core/src/adapters/outbound/text_merge.rs`: `DiffyMerge`, declared in `adapters/outbound/mod.rs`, the only file naming `diffy` (T042a enforces it)
- [X] T044 [US3] `client/core/src/application/use_cases/reconcile.rs` (declared in `use_cases/mod.rs`): `Reconcile::run` per design.md's sequence — read the host, compare against the base, write or merge or conflict, and delete the row only inside the transaction that commits the write. The base is the write protocol's existing `baseSha256` and no new protocol field is introduced (FR-027) Map the engine's `-32004` to `Conflicted`, per FR-020b: the host moved between this reconciliation's read and its write, which is a disagreement and not a failure.
- [X] T045 [US3] In `client/core/src/application/use_cases/reconcile.rs`, `ReconcileReport` and `Outcome`, returned rather than raised: a reconciliation that returned `Err` would lose the per-file detail FR-024 requires
- [X] T046a [US3] `client/core/src/application/use_cases/reconnect.rs` (declared in `use_cases/mod.rs`): the reconnection loop §11.5 specifies and nothing ran -- FR-018a. Behind a `Reconnectable` port the transport implements, so the loop is tested against a fake rather than by spawning processes. Uses `supervise::Backoff` and `supervise::response_to` as they stand: retry a transient failure after a jittered delay, stop on one that will not fix itself. Reports each wait through the transport's existing `report_retrying`, which emits the `Retrying` state the status bar already renders and which, until now, had no caller. Jitter from `std::collections::hash_map::RandomState` rather than a new dependency
- [ ] T046b [US3] In `client/core/src/composition.rs`, start the loop on a loss and run the reconnection sequence on success -- register the current workspace, refresh git status, **then** reconcile -- replacing T046's transition subscriber. That subscriber fired once at startup before any workspace existed and, with a working reconnection, would have raced the re-registration. `register_with_engine` returns whether the engine answered, so a workspace that cannot be registered is not reconciled (EC-16)
- [X] T046c [US3] `client/core/tests/reconnect.rs`: the loop retries a transient failure with growing waits, reports `Retrying` before each wait, stops on a host-key change without retrying, resets after success, and never runs twice at once. Plus the sequence's order, asserted as a recorded list: register before reconcile
- [ ] T046d [US3] In `client/ui/lib/shell/Window.svelte`, declare the watched paths again when the connection returns. A fresh engine has forgotten every watch, and a tree that stops updating while looking current is the state `workspace_resume`'s own comment calls the most misleading this application can be in
- [X] T046 [US3] **Its trigger is superseded by T046b; the construction stands.** In `client/core/src/composition.rs`, construct `DiffyMerge` and inject it as `Reconcile`'s `TextMerge`, then wire `Reconcile` to run once per transition into `Connected`, from the connection state already published (A-RECONNECT). Not a timer, not the first successful request. **Both halves**: T043 builds the adapter and this is the only place that hands it to anything, so without the construction the reconciler has a port and no implementation. Naming the **type** `DiffyMerge` here is correct and does not breach the confinement T042a enforces — that rule is about the `diffy` crate, and `engine/src/main.rs:87` sets the precedent by naming `inotify_watcher::git_watch()` in the composition root while `inotify_confinement.rs` strips that module's name before looking for the library
- [ ] T047 [US3] In `client/core/src/adapters/inbound/tauri_commands.rs` and `client/ui/lib/offline/state.svelte.ts`, surface the reconciliation report to the interface (FR-024)

**Checkpoint**: offline work reaches the host wherever it can do so unambiguously.

---

## Phase 6: User Story 4 — See a real conflict, and decide it myself (Priority: P2)

**Goal**: Where changes genuinely collide, the developer decides and the client writes nothing
until they do.

**Independent test**: Edit the same lines offline and on the host, reconnect, and confirm the
developer is asked and neither version is written until they answer.

### Tests for User Story 4

- [ ] T048 [US4] In `client/core/tests/reconcile.rs`, US4 scenarios 1 and 2 and FR-021, FR-033 and SC-006: overlapping changes prompt, **nothing** is written to the host, and the offline work is still retained while the developer has not answered
- [ ] T049 [US4] In `client/core/tests/reconcile.rs`, US4 scenario 6 and FR-025a and SC-006a: a file the client cannot merge prompts **even when the host did not change it**. This is the half an implementation would quietly drop, and the one the reviewer chose against the recommendation
- [ ] T050 [US4] In `client/core/tests/reconcile.rs`, US4 scenario 5 and FR-026: a file deleted on the host while edited offline prompts rather than letting the deletion or the edit win
- [ ] T051 [US4] In `client/core/tests/reconcile.rs`, US4 scenario 3 and `contracts/offline-commands.md` `conflict_resolve` guarantee 1: resolving writes the resolution and clears the pending row, in one transaction — neither without the other Also guarantee 3: with two conflicts outstanding, resolving one leaves the other's pending row untouched. FR-023 makes reconciliation per file and T037 tests that on the reconcile path; resolution is the other path to the same property, and a transaction scoped one row too wide would pass every single-conflict test.
- [ ] T052 [US4] In `client/core/tests/reconcile.rs`, `contracts/offline-commands.md` guarantee 2 on `conflict_resolve`: a write refused as stale becomes a **new conflict against the newer remote**, not an error the developer has to interpret
- [ ] T052a [US4] In `client/core/tests/merge_agreement.rs`, US4 scenario 7: two offline edits in different regions of one file, with a host change close to one of them, prompt **for that region on version-control merge terms** rather than for the file as a whole. The fixture must put the third change near one edit and far from the other, or the test cannot tell per-region from per-file
- [ ] T052b [US4] In `client/core/tests/reconcile.rs`, `contracts/offline-commands.md` `conflicts_list` guarantees 2 and 3: the remote side is **read when the list is built**, so changing the host between two listings changes what the second shows; and `base` is empty **only** for a file created offline, not for one whose cached content was evicted. The first guards against resolving against a remote that went stale while the developer was deciding — a silent wrong answer, not a failure
- [ ] T052c [US4] In `client/core/tests/reconcile.rs`, `contracts/offline-commands.md` `conflicts_list` guarantees 1 and 4: **all three versions are always present** — base, local and remote — and a file the client cannot merge is **still listed**, with `base` present and the reason stated. Guarantee 1 is what the conflict panel renders, so its absence makes T058 unable to show a conflict at all rather than showing it wrongly; guarantee 4 is the case an implementation drops because an unmergeable file has no merge result to report, which is exactly why it must still appear
- [ ] T053 [US4] `tests/e2e/offline-conflict.spec.ts`: US4 scenario 4 and FR-025 and SC-003 — a conflict left unresolved survives going offline again, and across the full cycle of edit, disconnect, relaunch, reconnect **zero** work is lost for any outcome including conflict and is presented on the next reconnection. The second disconnection is real and the conflict genuinely unresolved, not resolved and re-created
- [ ] T054 [US4] `tests/unit/offline-presentation.test.ts`: the conflict panel's three sides are labelled and distinguishable without colour, and a conflict with no cached base still renders

### Implementation for User Story 4

- [ ] T055 [US4] In `client/core/src/adapters/inbound/tauri_commands.rs`, `conflicts_list` per contract — the remote side read when the list is built, never stored, so it cannot go stale while the developer decides
- [ ] T056 [US4] In `client/core/src/adapters/inbound/tauri_commands.rs`, `conflict_resolve` — write and forget in one transaction, and a stale refusal becomes a fresh conflict
- [ ] T056a [US4] In `client/core/src/lib.rs`, add `cmd::conflicts_list` and `cmd::conflict_resolve` to `generate_handler!`, for the reason T019a records
- [ ] T057 [P] [US4] `client/ui/lib/offline/conflicts.svelte.ts`: `ConflictStore`, refreshed rather than cached, for the same staleness reason
- [ ] T058 [US4] `client/ui/lib/offline/ConflictPanel.svelte`: the three sides and the choice, from design-system tokens only. Build it from the accent and neutral ramps and `--color-divider`: the design system exposes **no semantic tone token** — no warning, error or success colour — and nothing in the product styles `data-tone`, so there is no precedent to follow and none is needed. A conflict panel's job is to show three versions, not to alarm. A raw hex colour is caught by `scripts/lint-ds.mjs`; `rgb()` and a named colour are not, so the rule to follow is the design system's, not the lint's. **Recorded as a deviation** in spec.md and plan.md: the prototype has no conflict screen, the same shape as F011's branch indicator
- [ ] T059 [US4] In `client/ui/lib/shell/Window.svelte`, surface outstanding conflicts where the developer will see them without hunting

**Checkpoint**: nothing can be silently lost or silently chosen.

---

## Phase 7: User Story 5 — Have what I need before I lose the connection (Priority: P3)

**Goal**: Going offline is not a lottery about which files happen to be cached.

**Independent test**: Work online without opening the manifests or recently changed files, go
offline, and confirm they are readable.

### Tests for User Story 5

- [ ] T060 [US5] `engine/tests/git_recent.rs`: parse `git log --name-only` output into a deduplicated path set — a file changed in five of twenty commits appears once (`contracts/recently-changed.md` guarantee 4)
- [ ] T061 [US5] In `engine/tests/git_recent.rs`, guarantee 3 and **F011's** FR-003a: a workspace on a **subdirectory** receives its own paths re-rooted and nothing from elsewhere in the repository, on the same terms `git/getStatus` gives
- [ ] T062 [US5] In `engine/tests/git_recent.rs`, guarantee 5: a directory that is not a repository, and a host without git, each answer successfully with an **empty list**
- [ ] T063 [US5] In `engine/tests/git_recent.rs`, guarantee 2: `commits` defaults to 20 and is capped at 100; a caller asking for more gets 100
- [ ] T063a [US5] In `engine/tests/git_recent.rs`, `contracts/recently-changed.md` guarantee 1: the result carries **paths only** — no file content, no commit identities, no authors, no dates. Asserted on the serialised **payload**, not on the parser, because a parser that discards the extra fields and a result that carries them look identical from the parser's side. F011's equivalent guarantee was untrue until exactly this test was written
- [ ] T063b [US5] In `engine/tests/git_recent.rs`, guarantee 6: a result that would exceed §4.1's frame cap is **truncated rather than refused**, and no cursor is offered. Truncation is not an error: prefetch is speculative and a partial answer is a partial prefetch. Driven with a repository whose recent commits genuinely touch more paths than one frame holds, or the branch never executes
- [ ] T064 [US5] `client/core/tests/prefetch.rs`: US5 scenarios 1 and 3 and FR-029, FR-032 and SC-010 — manifests and recent-commit files are cached without being asked for, asserted against prefetch's **own report** of what it fetched rather than against a wall-clock wait, and a workspace with no repository still caches manifests and does not fail
- [ ] T065 [US5] In `client/core/tests/prefetch.rs`, US5 scenario 5 and FR-029a and SC-010a: with the cache at its budget, prefetch **stops**, evicts nothing, and reports stopping rather than failing. The cache must genuinely be at its budget and the opened file genuinely least-recently-used, or the check passes for an implementation that evicts freely
- [ ] T066 [US5] In `client/core/tests/prefetch.rs`, US5 scenario 4 and FR-031: an interrupted prefetch leaves no half-written cache entry
- [ ] T067 [US5] `client/core/tests/prefetch_budget.rs`: US5 scenario 2, FR-030 and SC-009 **asserted and printed** — the test fails when interactive latency during prefetch exceeds its idle latency by more than 10%, and prints both (Principle V)

### Implementation for User Story 5

- [ ] T067a [US5] In `protocol/src/wire.rs`, add `RecentlyChangedParams` and `RecentlyChangedResult` per `contracts/recently-changed.md`, snake_case on the wire (A-WIRECASE), with `commits` optional
- [ ] T067b [P] [US5] `protocol/tests/recent_wire.rs`: hand-written JSON round-trips for both types, asserting the wire spelling rather than trusting the derive
- [ ] T068 [P] [US5] In `engine/src/application/ports/git.rs`, add `recently_changed` to the `Git` port
- [ ] T069 [US5] In `engine/src/adapters/outbound/git_cli.rs`, implement it with `git log --name-only --pretty=format: -n <commits> -- .`, honouring the prefix re-rooting F011's FR-003a established and the config pinning already there. **All three flags matter and were verified against git 2.43**: without `--pretty=format:` the parser meets commit headers; without `-- .` the walk covers the whole repository's history, and while the prefix strip would still drop the outsiders, the work is wasted on exactly the monorepo the scoping exists for; paths come back repository-root-relative, so the same prefix machinery as `status` applies unchanged
- [ ] T070 [US5] In `engine/src/adapters/inbound/rpc.rs`, dispatch `git/recentlyChanged`, resolving the workspace through `roots.resolve` exactly as the `git/getStatus` arm does. That one call yields both of the contract's workspace refusals — `-32001` for unregistered and **`-32009` for a registered workspace whose root is gone** (`ports/roots.rs:13`) — and a non-positive `commits` is refused with `-32602`. Resolve rather than hand-roll the registered check: this task originally named only `-32001`, which is what losing `-32009` looks like, and conflating a gone root with the contract's *successful empty answer* for a non-repository would let prefetch cache nothing and report nothing wrong
- [ ] T070a [US5] In `engine/tests/git_recent.rs`, the gone-root refusal: a registered workspace whose directory has been deleted is refused with **`-32009`**, not `-32001` and not a successful empty list. The two successes are the trap — guarantee 5 makes an empty list the right answer for a directory that is not a repository, so an implementation that answers empty for a *missing* directory passes every other test in this file. F011 promised this refusal in `git-status.md` and left it untested on that method; its resolver does produce it, so this test is the one F012 can afford to write while it is writing the arm
- [ ] T071 [P] [US5] In `client/core/src/application/ports/git_provider.rs` and `client/core/src/adapters/outbound/remote_git.rs`, add `recently_changed` through the transport
- [ ] T072 [US5] `client/core/src/application/use_cases/prefetch.rs` (declared in `use_cases/mod.rs`): `Prefetch::run` — manifests first, then recent-commit paths, checking the budget before each fetch and stopping rather than evicting. It **returns** design.md's `PrefetchReport` — how many it fetched, and whether it stopped at the budget — for the reason T045 states about `ReconcileReport`: SC-010 is measured against what prefetch says it fetched, and SC-010a against whether it stopped, so a run that logged instead of returning leaves both unmeasurable while looking finished
- [ ] T073 [US5] In `client/core/src/composition.rs`, invoke `Prefetch::run` on **workspace open while connected** and on **transition into `Connected`** (FR-029b), at background priority (§4.6) so it cannot delay interactive traffic (FR-030). Both triggers, not one: open alone never prefetches a workspace that was open before the connection returned, and reconnect alone never prefetches the first workspace of a session. No idleness detector — §4.6 already orders background behind interactive and bounds the starvation, so detecting idleness here would be the second authority Principle II refuses
- [ ] T073a [US5] In `client/core/tests/prefetch.rs`, FR-029b: prefetch runs for a workspace opened **after** the application started, and runs again on a reconnection, and does **not** run while disconnected. The first case is the one a startup-only implementation fails, and it fails silently — the cache is merely emptier than expected, which no other US5 test notices

**Checkpoint**: all five subfeatures delivered.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [ ] T074 [P] Update `docs/app-shell.md` with the client side: why a pending edit is its own table and outlives its cache entry, why the row is the durable fact and every outcome a statement about one attempt, and why the remote side of a conflict is never stored
- [ ] T075 [P] Update `docs/engine.md` with `git/recentlyChanged`: paths only, capped, not paged, and why a cursor would be the opposite of a bounded prefetch
- [ ] T076 Per `quickstart.md` §5, run **every** mutation it lists and record each outcome in §8. Each must fail **with the assertion expected** rather than with a compile error. §5 states which matter most and why; that ranking is not repeated here, because it changed twice while this line said otherwise — the count and the ranking live in §5 alone
- [ ] T077 Per `quickstart.md` §4, audit **every** negative check it lists and confirm for each that the condition which lets it fail is actually present — a real binary rather than a text file named `.bin`, a genuinely untouched host side, a cache genuinely at its budget, an interruption genuinely between two files
- [ ] T078 In `quickstart.md` §6, record **every** measurement §3 lists in the *Validation record*, each number beside its bound. A gate that says only PASS tells nobody how much headroom is left
- [ ] T079 Confirm every one of spec.md's 29 acceptance scenarios is named by at least one test, and every FR and SC is cited by name in a test file. F011 shipped with three requirements tested but uncited, which made a coverage audit read them as gaps
- [ ] T079a Run `npm run e2e:live` and record its wall-clock time beside the figure `wdio.conf.ts` already carries — eight files in 46 seconds. The offline specs disconnect and restart the engine, which is the same class of thing `terminal-live.spec.ts` does, and that config's comment says plainly that what `terminal-live` leaves behind "has not been identified". Three new specs now run before it. If the suite slows by minutes rather than seconds, the residue is ours and the ordering hazard has a second instance; report it rather than reordering until it passes, because "green in this order" is the weaker claim the comment exists to keep visible
- [ ] T080 Verify `make gate` is green, then mark F012's five subfeatures in `specs/features-map.md` and run `feature_map.py verify`. `make gate` includes `gate:fidelity`, which compares pixels against `tools/gate-fidelity/reference/baseline-1200x800.png`. **If it goes red, do not run `gate:fidelity:update` to clear it.** Say first which pixels moved and why: the baseline was last changed by F010, which added a visible surface, and F011's status-bar branch indicator needed no change because the baseline's profile has no engine and a workspace with no remote is never offline (§13.1) — so F012's indicator should be invisible there too, and a red gate means that reasoning is wrong rather than that the baseline is stale. Updating blind accepts every other visual change in the same commit, which is the one thing a pixel baseline exists to prevent. The three `SURFACES` in `derive.mjs` are the shell's frame and need no entry for the conflict panel, which is content inside one of them

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
  because it is P3; it is the one story that degrades rather than breaks the feature by its
  absence. Measured rather than asserted, since this is the claim a decision to defer US5 would
  rest on: of its eighteen tasks, exactly one writes a file another story owns — T073, in
  `composition.rs`, which every story necessarily touches — and no task outside US5 touches any
  file US5 owns. Deferring it removes eighteen tasks and one line of wiring.

### Within Each User Story

Tests before the implementation they cover, in every phase. The order is the project's practice
and Principle VII's requirement, and it is what made F011's defects visible before they shipped.

### Parallel Opportunities

**What `[P]` means here.** Exactly what the convention says: the task writes a file no other task
writes. Ten tasks carry it. `scripts/pipeline.py`'s `parallel_collisions` enforces it, and its
docstring records why — two such collisions shipped in F003's task list and survived an analysis
pass that never cross-referenced paths against the marker.

**This paragraph previously said something else**, and was wrong. It claimed `[P]` meant
"independent *as work*" and did not license two writers in one file, which let thirty-odd tasks
keep the marker while sharing a test file. That is a second meaning for a marker the tooling
already defines — the second authority Principle II exists to refuse — and `pipeline.py`
reported ten collisions the moment run 28 finally ran it. The markers are gone from every task
that shares a file.

The information they were carrying is true and belongs here instead: most tasks in this list write
a file another task also writes, because a test file gathers a story's cases — `reconcile.rs`,
`retain_edit.rs` and `git_recent.rs` each gather many. No numbers: pass 28 wrote three here and pass
30 found one of them already wrong, in the paragraph that forbids exactly that. Those cases are
independent of each other, which is what matters when deciding what to leave until later. Being
independent is not the same as being safe to write concurrently, and only the second is what `[P]`
claims.

**Genuinely parallel — different files, no shared writer:**

- Protocol, schema and port work in Phase 2: different crates.
- US5's engine tests against any client story's work: different crates.
- The two documentation tasks in Phase 8: different documents.

**Files with more than one writer** must be taken one task at a time. There is no list here,
because there no longer needs to be one: after analysis pass 28 stripped `[P]` from every task
sharing a file, **the absence of `[P]` is the list.** A task without the marker shares its file
with another task; a task with it owns its file alone, and `scripts/pipeline.py`'s
`parallel_collisions` will say so if that ever stops being true.

The previous version of this section was a table of files with the number of writers spelled out
beside each. By pass 30 four of its eight counts were wrong and six multi-writer files were missing
from it, which is what the standing note above forbids — a count restated outside the document
that derives it. Its own closing line congratulated it for describing files rather than task ids,
"because an enumerated id list goes stale on the next insertion". The counts went stale on exactly
the same insertions.

## Parallel Examples

There are none written out here any more. Every previous version was a snapshot: the User Story 1
example announced "all six tests for User Story 1" above a list of four, named T018 and T019 as
"the only independent ones" after T022 had become a second writer of `state.svelte.ts`, and went
stale on every insertion the twenty-nine analysis passes made.

The rule replaces them, and it is now readable off each task: **`[P]` means the task owns its file,
so any set of `[P]` tasks in one phase may be worked at once.** Everything else is taken one task at
a time per file. `scripts/pipeline.py`'s `parallel_collisions` enforces it, so the marker cannot
drift from the truth the way a hand-written example did.

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
