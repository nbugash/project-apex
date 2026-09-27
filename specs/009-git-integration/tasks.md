---

description: "Task list for F011 git-integration"
---

# Tasks: Git Integration

**Input**: Design documents from `/specs/009-git-integration/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/git-status.md](./contracts/git-status.md),
[architecture.md](./architecture.md), [design.md](./design.md)

**Tests**: Included and **not optional**. Constitution Principle VII requires every feature to
ship with tests at every level; the template's "only if requested" does not apply to this project.

**Organization**: Grouped by user story, so each is an independently testable slice.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel — different files, no dependency on incomplete work
- **[Story]**: US1–US4, on story-phase tasks only

## Path Conventions

Paths follow [plan.md](./plan.md)'s Structure Decision and [design.md](./design.md)'s Module &
File Layout: `protocol/`, `engine/`, `client/core/`, `client/ui/lib/`, `tests/` at the repository
root.

---

## Phase 1: Setup (Shared Infrastructure)

- [X] T001 Add a scratch-repository fixture builder in `engine/tests/common/repo.rs` that creates a real repository with a modified file, a staged file, an untracked file, a deleted file, a rename and a conflict — the six shapes research.md verified. Every engine git test builds from this rather than shelling out ad hoc
- [X] T002 [P] In `engine/tests/common/repo.rs`, extend the fixture builder to produce a **linked worktree**, where `.git` is a file, because FR-004's whole point is that this case differs and a fixture that cannot produce it cannot test it
- [X] T003 [P] Register the git end-to-end specs in the live run in `tests/e2e/wdio.conf.ts`, alongside the editor specs and before `terminal-live.spec.ts`, which must stay last for the reason recorded there

---

## Phase 2: Foundational (Blocking Prerequisites)

**These block every user story. Nothing in Phase 3 onward can start until they land.**

- [X] T004 `protocol/src/wire.rs`: `GitStatusParams`, `GitStatusResult`, `GitChange`, `GitDiffParams`, `GitDiffResult` and the five status values, spelled `snake_case` on the wire
- [X] T005 `protocol/tests/git_wire.rs`: assert the wire spelling against **hand-written JSON**, not a round trip of the structs. A round trip is symmetric and agrees with itself; only a frame written by hand checks the convention A-WIRECASE records. Include a camelCase frame that must be refused
- [X] T006 [P] `engine/src/application/ports/git.rs`: the `Git` port — `status`, `file_diff`, `git_dir` — and `GitFailure` with `NotARepository`, `GitUnavailable` and `Failed`, which the design keeps distinct inside the engine even though the wire collapses the first two
- [X] T007 `client/core/src/adapters/outbound/sqlite/schema.rs`: re-key git state from the tree's file identity to (workspace, path), add the per-workspace branch, raise `CURRENT_VERSION`
- [X] T008 `client/core/tests/migrate_git_status.rs`: a store at the previous version opens at the new one with its other content intact. Nothing has ever written a git row, so the assertion is that the *rest* of the projection survives — a migration test that only checked the new table would pass for one that dropped the database

---

## Phase 3: User Story 1 — See which files I have changed (Priority: P1) 🎯 MVP

**Goal**: A change on the host is marked in the tree, within two seconds, with no request issued
to render it.

**Independent test**: Open a workspace that is a repository, change a file outside the client,
and confirm the tree marks that file and no other.

### Tests for User Story 1

- [X] T009 [P] [US1] `engine/tests/git_parse.rs`: parse captured `--porcelain=v2 -z` output for all six fixture shapes. **A rename must not desynchronise the parser** — its original path is a separate NUL field, and a parser that split on NUL and took one record per field reads it as a new record (research.md) (FR-007)
- [X] T010 [P] [US1] In `engine/tests/git_parse.rs`, the state precedence: a conflict beats everything, an unstaged character beats a staged one, and `.M` and `M.` produce different results. Asserted on the pair of characters rather than on one, because collapsing them early is the mistake this rule exists to prevent (FR-008)
- [X] T011 [P] [US1] `engine/tests/git_degrade.rs`: a workspace that is **not a repository** and a host where git **cannot be run** each produce a successful, empty status with no branch — not an error (FR-027, FR-028). Both kinds are asserted, because the engine keeps them distinct internally and only a test says whether the collapse to one client-visible answer actually happens
- [X] T012 [P] [US1] `engine/tests/git_watch.rs`: a real `git add` — which touches the index and no working-tree file — fires the watch. This is the case the workspace watcher cannot see and the reason A-GITWATCH exists (FR-002, FR-003)
- [X] T013 [US1] In `engine/tests/git_watch.rs`, a watch established on the **linked worktree** fixture fires too, proving `git_dir` resolution rather than a `<root>/.git` assumption (FR-004)
- [X] T014 [US1] In `engine/tests/git_watch.rs`, assert the git watch's events produce **no** `workspace/onFileEvent`. A-GITWATCH's separation is only worth its extra component if this holds, and only a test says whether it does (FR-005)
- [X] T014a [US1] `engine/tests/git_push.rs`: the engine **speaks first** — a plain save to a tracked file, and a `git add`, each produce one `git/onStatusUpdate` within two seconds with no request issued in between (FR-002, FR-002a, FR-003, SC-001, SC-002). Every other test in this feature inspects a reply, and an engine that only ever answered would pass all of them while failing what the feature is for
- [X] T015 [P] [US1] `engine/tests/git_coalesce.rs`: fifty index writes inside one second produce **one** status computation, against a fake clock. Count computations, not notifications — a coalescer that emits once while running git fifty times passes the wrong assertion (FR-006b, SC-013)
- [X] T016 [US1] In `engine/tests/git_coalesce.rs`, a change arriving *during* a run schedules exactly one re-run, so a burst of any length costs two computations rather than N
- [X] T017 [P] [US1] `engine/tests/git_status_paging.rs`: a repository with 5,000 changed paths is served completely across pages, every page holds at most 1000, and the final page alone carries no cursor (FR-006a, SC-012)
- [X] T018 [US1] In `engine/tests/git_status_paging.rs`, pages of one cursor chain describe **one snapshot**: changing the repository mid-pull does not change what later pages report. This is the guarantee a per-page re-run would break invisibly
- [X] T019 [P] [US1] `client/core/tests/git_apply.rs`: an update whose first page carries a cursor is **not** applied until the final page arrives, and an interrupted pull leaves the previous state exactly as it was. The failing implementation here passes every single-message test (A-GITPAGE) (FR-009, FR-009a, SC-015)
- [X] T020 [P] [US1] In `client/core/tests/git_apply.rs`, applying any number of updates leaves the count of cached files and their hashes unchanged (FR-010, §5.3) (FR-010, SC-006)
- [X] T021 [P] [US1] `client/core/tests/git_apply.rs`: an update naming an unknown workspace is discarded, and one workspace's update never alters another's rows (FR-011, FR-012)
- [X] T022 [P] [US1] `client/core/tests/git_apply.rs`: an untracked file in a folder the tree has **never listed** is stored and readable. This is the test that justifies keying by path (FR-009b, SC-014) — the tree-keyed design F003 shipped passes every other test in this phase and fails only this one
- [X] T023 [P] [US1] `client/core/tests/git_persist.rs`: git state written by one client session is present when a new session opens the same workspace, without a refresh having arrived (FR-013). Asserted on a reopened store rather than on a live one, because an in-memory projection satisfies every other assertion here
- [X] T024 [P] [US1] `client/core/tests/git_offline.rs`: with no connection, the last applied git state remains readable and is presented on the same terms as other content that cannot be confirmed, rather than clearing (FR-029). A cleared projection would tell the developer nothing had changed, which is the one thing it must not say when it does not know
- [X] T025 [P] [US1] `tests/unit/git-marker.test.ts`: every state maps to a token the design system defines, and all five are distinguishable **in greyscale** — compared by luminance, following `rail-greyscale.spec.ts`, because comparing hues passes for a design that carries the whole distinction in colour (FR-014, FR-015, SC-008)

### Implementation for User Story 1

- [X] T026 [US1] `engine/src/adapters/outbound/git_cli.rs`: `git_dir` via `rev-parse --git-dir`, resolving a `.git` file to the real directory
- [X] T027 [US1] In `engine/src/adapters/outbound/git_cli.rs`, the `--porcelain=v2 -z --branch` invocation and a parser that dispatches on record type — `1`, `2`, `u`, `?`, `!`, `#` — consuming the extra path field for `2`
- [X] T028 [US1] In `engine/src/adapters/outbound/git_cli.rs`, map the two status characters to one state per data-model.md's derivation rule, and drop any entry whose path escapes the workspace root rather than forwarding it (Principle VI)
- [X] T029 [US1] In `engine/src/adapters/outbound/inotify_watcher.rs`, two inotify watches on `HEAD` and `index` in the resolved git directory, on **its own inotify instance**, with no reference to the exclusion set — the separation that makes T014 structurally true. Not a new `git_watch.rs`: `inotify_confinement.rs` permits the library to be named in exactly one file, and a second one would dilute the rule that keeps every watcher decision in a place tests can reach
- [X] T029a [US1] `engine/src/application/ports/git_watch.rs`: a `StatusNudge` port carrying a workspace and **no event detail**, called from `engine/src/adapters/outbound/watch_thread.rs` where workspace file events are emitted. The two watches are necessary and not sufficient: an ordinary save writes neither `HEAD` nor `index` (A-GITNUDGE, FR-002a). One direction only — nothing travels git-to-workspace, which is what keeps T014's structural guarantee true
- [X] T029b [US1] `engine/src/adapters/outbound/git_watchers.rs`: the component that connects the watch, the coalescer, the pager and the writer, and emits `git/onStatusUpdate`. Without it every part of this phase exists and nothing reaches a client — F004's notification with no caller, repeated
- [X] T030 [US1] `engine/src/application/use_cases/git_status.rs`: the coalescer — 100 ms trailing edge, at most one computation in flight, at most one queued (plan.md, *Fixed Quantities*)
- [X] T031 [US1] In `engine/src/application/use_cases/git_status.rs`, the pager: hold one computed snapshot per workspace against an opaque cursor, serve ≤1000 per page, discard when the last page is served or the client goes away
- [X] T032 [US1] In `engine/src/adapters/inbound/rpc.rs`, dispatch `git/getStatus` and emit `git/onStatusUpdate` carrying the branch and the first page, refusing an unknown or cross-workspace cursor with invalid-params rather than treating it as the beginning
- [X] T033 [P] [US1] `client/core/src/application/ports/git_provider.rs`: the `GitProvider` port, so the consumer cannot tell whether an engine is on the other side
- [X] T034 [US1] `client/core/src/adapters/outbound/remote_git.rs`: `GitProvider` over the transport, mapping §4.4 codes to typed errors so no caller parses a message
- [X] T035 [US1] `client/core/src/application/use_cases/apply_git_status.rs`: accumulate pages, commit the replacement in one transaction when the final page lands, discard on any failure, and never touch cache validity
- [X] T036 [US1] In `client/core/src/application/ports/workspace_cache.rs` and the SQLite adapter, read git state by workspace and replace it wholesale
- [X] T037 [US1] In `client/core/src/adapters/inbound/tauri_commands.rs`, `git_status` reading the projection, with the workspace resolved in the core rather than accepted from the webview (Principle VI, as F010 and F006 both do)
- [X] T038 [US1] `client/ui/lib/git/status.svelte.ts`: the projection the surfaces read, updated by a subscription rather than polled
- [X] T039 [P] [US1] `client/ui/lib/git/marker.ts`: state to design-system token and glyph, pure. Tokens read from the element, no colour literal — `lint:ds` refuses one, which is how F006's first new surface was caught inventing token names
- [X] T040 [US1] `client/ui/lib/workspace/FileTree.svelte`: fill the `.vcs` column F000 reserved, using `--vk-tree-vcs-size`, so adding it reflows nothing. A file with no git state keeps exactly the row it has today (FR-016), which the existing workspace-tree specs already assert and which must still hold after this change
- [X] T041 [US1] `client/ui/lib/shell/Window.svelte`: subscribe to status updates once for the window, as the file-event router is, so a background tab is covered
- [ ] T042 [US1] `tests/e2e/git-status.spec.ts`: **all six** of US1's acceptance scenarios against a real engine and a real repository — a modified file marked, a staged file's mark changing, an untracked file distinguishable without colour, **zero** requests to render a folder, the cached-file count unchanged across an update, and **the same files still marked after a relaunch** (FR-013). Each mark asserted to arrive **within 2 seconds** of the host change (SC-001, SC-002), so the bound is measured where the behaviour is rather than only tabulated in quickstart. Six, counted against spec.md, because Principle VII makes each scenario's test an obligation and the sixth was missing when analyze checked

---

## Phase 4: User Story 2 — Know which branch I am on (Priority: P2)

**Goal**: The status bar names the branch, and says nothing rather than something wrong when
there is no branch to name.

**Independent test**: Open a repository and confirm the status bar names its branch; switch on
the host and confirm it follows.

### Tests for User Story 2

- [ ] T043 [P] [US2] `engine/tests/git_branch.rs`: `# branch.head master` yields the branch; `# branch.head (detached)` yields the **detached case and not a branch named "(detached)"**, which is what a header-as-name reading produces (research.md) (FR-018, FR-019)
- [ ] T044 [P] [US2] In `engine/tests/git_branch.rs`, a repository with no commits reports its branch name with `(initial)` as the commit, so an unborn branch shows a name rather than crashing on a missing object
- [ ] T045 [P] [US2] `tests/unit/git-branch.test.ts`: the three cases render as a name, a short commit, and nothing at all — never an empty label or a placeholder (FR-019, SC-009)

### Implementation for User Story 2

- [ ] T046 [US2] In `engine/src/adapters/outbound/git_cli.rs`, parse the branch header into the three-case position data-model.md defines
- [ ] T047 [US2] In `client/core/src/application/use_cases/apply_git_status.rs` and the cache, store the branch per workspace alongside the state replacement
- [ ] T048 [US2] `client/ui/lib/statusbar/StatusBar.svelte`: the branch indicator, built from design-system tokens only. This surface is **not in the prototype** and is recorded as a deviation in spec.md; a designer must be able to move it without unpicking an improvised value
- [ ] T049 [US2] `tests/e2e/git-branch.spec.ts`: the branch shows, follows a switch, is absent for a non-repository, and identifies the commit when HEAD is detached. For the non-repository case also assert the workspace stays fully usable and surfaces **zero** errors (SC-007) — the engine's half of that is T011's, and a successful empty status still reaches a client that could render it as a failure

---

## Phase 5: User Story 3 — See which lines I changed (Priority: P2)

**Goal**: Opening a modified file marks the changed lines, without the file's previous contents
ever crossing the wire.

**Independent test**: Open a modified file and confirm the gutter marks added, changed and
deleted lines; open an unmodified one and confirm it marks none.

### Tests for User Story 3

- [ ] T050 [P] [US3] `engine/tests/git_diff.rs`: parse `--unified=0` hunk headers, including **`@@ -2 +2 @@` where the counts are elided** and `@@ -4,0 +5 @@` where a zero count means an insertion. A parser assuming two numbers per side mishandles the first (research.md) (FR-020)
- [ ] T051 [P] [US3] In `engine/tests/git_diff.rs`, assert the result carries **no file content** — inspected on the payload, not on the parser, because a parser that discards text and a result that carries it look identical from the parser's side (FR-021, SC-010)
- [ ] T052 [P] [US3] In `engine/tests/git_diff.rs`, an unmodified file returns three empty lists rather than an error, and an untracked file returns every line as added (FR-022)
- [ ] T053 [P] [US3] `tests/unit/git-gutter.test.ts`: coordinates become decorations, a deletion becomes a position rather than a zero-length range, and an empty diff produces no decorations (FR-020, FR-023)
- [ ] T054 [US3] `client/core/tests/git_gutter_budget.rs`: p99 over at least 100 samples from opening a modified file to its coordinates being available, **printed** against 250 ms with the measured value shown (SC-004, A-NFR) (SC-004)

### Implementation for User Story 3

- [ ] T055 [US3] In `engine/src/adapters/outbound/git_cli.rs`, `file_diff` via `--unified=0`, taking only the hunk headers
- [ ] T056 [US3] In `engine/src/adapters/inbound/rpc.rs`, dispatch `git/getFileDiff`, refusing an escaping path with `-32002` and a missing one with `-32003`
- [ ] T057 [US3] In `client/core/src/adapters/outbound/remote_git.rs` and `tauri_commands.rs`, `file_diff` through to a `git_file_diff` command
- [ ] T058 [P] [US3] `client/ui/lib/git/gutter.ts`: coordinates to Monaco decorations, pure, so the mapping is testable without a window
- [ ] T059 [US3] `client/ui/lib/editor/EditorPanel.svelte`: apply the decorations on open, and follow a change to the file's git state
- [ ] T060 [US3] `tests/e2e/git-gutter.spec.ts`: US3's acceptance scenarios — marks for each kind, none on an unmodified file, marks following a host change, and zero file content in the diff payload

---

## Phase 6: User Story 4 — Switch branches without the client falling behind (Priority: P3)

**Goal**: A branch switch costs one invalidation, not one event per file, and leaves nothing
marked from the branch the developer left.

**Independent test**: Switch a branch that changes many files; confirm one bulk invalidation and
no stale marks.

### Tests for User Story 4

- [ ] T061 [P] [US4] `engine/tests/git_branch_switch.rs`: a switch changing 10,000 files produces **exactly one** invalidation. The count is the assertion — an implementation emitting per-file events satisfies every other test in this feature (FR-024, SC-005)
- [ ] T062 [P] [US4] `client/core/tests/git_invalidate.rs`: a bulk invalidation clears the workspace's git state and does **not** eagerly refetch the tree, and no file remains marked from the previous branch (FR-025, FR-026, SC-011)

### Implementation for User Story 4

- [ ] T063 [US4] In `engine/src/application/use_cases/git_status.rs`, a `HEAD` change emits `workspace/invalidateAll` once (§12.4) rather than a status of every path
- [ ] T064 [US4] In `client/core/src/application/use_cases/apply_git_status.rs`, apply a bulk invalidation by clearing git state for the workspace without touching cached content
- [ ] T065 [US4] `tests/e2e/git-branch-switch.spec.ts`: a real branch switch on the host leaves the tree showing the new branch's state and nothing from the old

---

## Phase 7: Polish & Cross-Cutting Concerns

- [ ] T066 [P] Update `docs/engine.md` with the git path: why two watches and not the workspace watcher, why a separate service, what the coalescer bounds, and why the pager holds a snapshot
- [ ] T067 [P] Update `docs/app-shell.md` with the client side: where git state lives and why it is keyed by path, and why an update commits on its last page
- [ ] T068 Per `specs/009-git-integration/quickstart.md` §5, run the eight mutation checks and record each outcome. Each must fail **with the assertion expected** rather than with a compile error. Mutations 2 and 4 matter most — each is a plausible simplification that leaves every other test green
- [ ] T069 Per `specs/009-git-integration/quickstart.md` §4, audit the eight negative checks and confirm, for each, that the condition which lets it fail is actually present — a real worktree rather than a directory, a real rename, a real burst, genuinely cached files
- [ ] T070 In `specs/009-git-integration/quickstart.md`, record the seven measurements from §3 in the *Validation record*, each number beside its bound. A gate that says only PASS tells nobody how much headroom is left
- [ ] T071 Verify `make gate` is green, then mark F011's six subfeatures in `specs/features-map.md` and run `feature_map.py verify`

---

## Dependencies & Execution Order

### Phase Dependencies

```text
Phase 1 Setup ──▶ Phase 2 Foundational ──┬──▶ Phase 3 US1 ──▶ Phase 6 US4
                                          ├──▶ Phase 4 US2
                                          └──▶ Phase 5 US3
                                                    │
                                        all ────────┴──▶ Phase 7 Polish
```

### User Story Dependencies

- **US1** depends only on Foundational. It is the MVP and the only story that must ship.
- **US2** depends on Foundational and on US1's parser reaching the branch header — in practice
  T046 extends T027, so US2 follows US1 rather than running truly beside it.
- **US3** is genuinely independent of US1: the diff path shares the `Git` port and nothing else.
  It can be built by a second person while US1 is in progress.
- **US4** depends on US1, because there must be state to invalidate before invalidating it means
  anything.

### Within Each User Story

Tests precede the implementation they describe. Inside the implementation, the order is engine →
protocol dispatch → client core → webview, because each consumes the last.

### Parallel Opportunities

- **Phase 1**: T002 and T003 are independent of T001 and of each other.
- **Phase 2**: T006 is independent of T004/T005 and of T007/T008.
- **US1 tests**: T009, T010, T012, T015, T017, T019, T020, T021 and T025 touch nine different
  files and can be written together.
- **US1 implementation**: T033 and T039 are independent of the engine work; the rest is a chain.
- **US3**: T050, T051, T052 and T053 are parallel; T058 is independent of the engine path.
- **Polish**: T066 and T067 are parallel; T068 through T071 are sequential by nature.

---

## Implementation Strategy

**MVP is US1 alone.** A developer who can see which files they have changed has the feature's
value; branch, gutters and invalidation refine it. US1 is also the slice that proves the whole
pipeline — watching, coalescing, parsing, paging, accumulating and drawing — so if it works, the
remaining stories are additions rather than discoveries.

**Build order for one person**: Phases 1–2, then US1 to a working tree marker, then US2 (small,
and it reuses US1's parser), then US3, then US4. Ship after US1 if time runs out; the other three
degrade to absent rather than broken.

**The two tasks most likely to be got wrong quietly** are T019 and T013. T019 is the paged
transaction: an implementation that applies the first page passes every test written against a
single message. T013 is the worktree: an implementation assuming `<root>/.git` works perfectly on
every ordinary repository and silently never updates on a worktree. Both have a test here for
that reason, and both appear in quickstart.md's mutation list.
