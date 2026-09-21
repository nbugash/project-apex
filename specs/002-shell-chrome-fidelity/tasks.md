---
description: "Task list for the Shell Chrome Fidelity feature"
---

# Tasks: Shell Chrome Fidelity

**Input**: Design documents from `/specs/002-shell-chrome-fidelity/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, architecture.md, design.md

**Tests**: Included and **not optional**. Constitution Principle VII requires unit, integration
and end-to-end coverage. The Linux-only end-to-end limitation is justified project-wide in
Appendix A, A-E2E and is cited rather than re-argued.

**Screenshots**: The end-to-end harness already captures every test to
`reports/screenshots/${os}/` through its `afterTest` hook, so every spec added here inherits
capture for free — no task rebuilds it. What is missing, and is added below, is anything that
*asserts* a capture contains a window. In F000 a blank capture was the only evidence that the
application was never visible; every other gate passed. That check is now automated rather
than left to someone noticing.

**Organization**: Grouped by user story. US1 and US2 are both P1; US1 leads because surfaces
must exist before navigation between them means anything.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on incomplete work)
- **[Story]**: US1–US3, mapping to the user stories in spec.md

## Path Conventions

Extends the trees F000 established. New: `tools/gate-fidelity/`, which sits outside the
application boundary and never ships.

---

## Phase 1: Setup

- [ ] T001 Extend the design system sync to write the prototype's layout dimensions to `src/lib/ds/layout-tokens.css` in `scripts/ds-sync.mjs` (FR-009). The path is inside the design-system directory deliberately: `lint:ds` skips only `src/lib/ds`, and a token file necessarily contains raw pixel values, so anywhere else fails the lint it is meant to satisfy
- [ ] T002 Import the generated layout tokens alongside the design system in `src/app.css` (FR-009)
- [ ] T003 [P] Add `gate:fidelity` and `gate:fidelity:update` scripts in `package.json` (FR-012, FR-016)
- [ ] T004 [P] Add the fidelity gate as a required check in `.github/workflows/ci.yml` (FR-012)
- [ ] T005 [P] Publish the end-to-end screenshot directory as a CI artifact on failure in `.github/workflows/ci.yml`, so a failed run's captures are reviewable without reproducing locally

**Checkpoint**: Prototype dimensions are tokens; a literal is now a lint failure

---

## Phase 2: Foundational (Blocking Prerequisites)

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

### Domain

- [ ] T006 Define `DestinationId`, `Availability` and `RailDestination` in `src-tauri/src/domain/rail.rs` (FR-002, FR-008)
- [ ] T007 Define `ToolWindowState` with select, resize and repair rules in `src-tauri/src/domain/rail.rs` (FR-004, FR-006)
- [ ] T008 Define `RailCatalogue` holding the static destination set in `src-tauri/src/domain/rail.rs` (FR-002)
- [ ] T009 [P] Unit tests for destination ordering, uniqueness and unavailable-destination behaviour in `src-tauri/src/domain/rail.rs` (FR-008)
- [ ] T010 [P] Unit tests for collapse toggling and width retention across collapse in `src-tauri/src/domain/rail.rs` (FR-006)

### Session migration — the riskiest change in this feature

- [ ] T011 Extend `PersistedSession` with tool window state and raise the schema version in `src-tauri/src/domain/session.rs` (FR-011)
- [ ] T012 Implement forward migration — fill defaults for older versions, discard only newer ones — in `src-tauri/src/domain/session.rs`
- [ ] T013 [P] Unit tests asserting an older version migrates, the current loads unchanged, and a newer version is discarded, in `src-tauri/src/domain/session.rs`
- [ ] T014 Integration test asserting a real older session file keeps its window geometry, layout and open tabs through the upgrade, in `src-tauri/tests/session_migration.rs`

### Application and adapters

- [ ] T015 Extend the persist use case with destination selection and tool window resize in `src-tauri/src/application/use_cases/persist_session.rs` (FR-004, FR-011)
- [ ] T016 Add `rail_select`, `tool_window_resize` and `rail_destinations` with boundary validation in `src-tauri/src/adapters/inbound/tauri_commands.rs` (FR-004)
- [ ] T017 [P] Unit tests asserting unknown destination identifiers are rejected at the boundary in `src-tauri/src/adapters/inbound/tauri_commands.rs`
- [ ] T018 Add typed wrappers for the three rail commands in `src/lib/ipc.ts`
- [ ] T019 [P] Implement rail ordering and keyboard-navigation helpers in `src/lib/rail.ts` (FR-007)
- [ ] T020 [P] Unit tests for rail ordering and neighbour resolution in `tests/unit/rail.test.ts` (FR-007)

**Checkpoint**: An older session survives the upgrade; rail state is drivable from the interface

---

## Phase 3: User Story 1 - Recognise the application as the approved design (Priority: P1) 🎯 MVP

**Goal**: Chrome header, activity rail and tool window render at the prototype's dimensions, from tokens.

**Independent Test**: Place the running application beside the prototype at 1200×800 and compare the three surfaces.

### Tests for User Story 1

- [ ] T021 [P] [US1] End-to-end test asserting the three chrome surfaces are present at the prototype's dimensions in `tests/e2e/chrome-fidelity.spec.ts` (SC-001)
- [ ] T022 [P] [US1] End-to-end test asserting fixed-width surfaces keep their widths as the window resizes in `tests/e2e/chrome-resize.spec.ts` (FR-010)
- [ ] T023 [P] [US1] End-to-end test asserting no chrome dimension resolves to a literal rather than a token in `tests/e2e/chrome-tokens.spec.ts` (SC-002)

### Implementation for User Story 1

- [ ] T024 [P] [US1] Implement the chrome header with product mark and project switcher in `src/lib/chrome/ChromeHeader.svelte` (FR-001)
- [ ] T025 [P] [US1] Implement a single rail destination button with active and unavailable states in `src/lib/chrome/RailButton.svelte` (FR-005, FR-008)
- [ ] T026 [US1] Implement the activity rail rendering its destinations in `src/lib/chrome/ActivityRail.svelte` (FR-002, depends on T025)
- [ ] T027 [P] [US1] Implement the tool window frame and header row in `src/lib/chrome/ToolWindow.svelte` (FR-003)
- [ ] T028 [US1] Compose chrome, rail and tool window into the shell in `src/lib/shell/Window.svelte` (depends on T024, T026, T027)

**Checkpoint**: The chrome is visible and matches the prototype by eye. MVP.

---

## Phase 4: User Story 2 - Move between tool windows from the rail (Priority: P1)

**Goal**: Selecting a destination switches the tool window, toggles collapse, survives restart, works by keyboard.

**Independent Test**: Select each destination; confirm the tool window changes and active state moves. Collapse and restore. Repeat by keyboard, then in greyscale.

### Tests for User Story 2

- [ ] T029 [P] [US2] End-to-end test for destination switching and active-state movement in `tests/e2e/rail-navigation.spec.ts` (FR-004, FR-005)
- [ ] T030 [P] [US2] End-to-end test asserting selecting the active destination collapses and restores at the previous width in `tests/e2e/rail-collapse.spec.ts` (FR-006)
- [ ] T031 [P] [US2] End-to-end test asserting every destination is reachable and activatable by keyboard alone in `tests/e2e/rail-keyboard.spec.ts` (FR-007, SC-003)
- [ ] T032 [P] [US2] End-to-end test asserting active and unavailable states stay distinguishable in greyscale in `tests/e2e/rail-greyscale.spec.ts` (FR-005, SC-004)
- [ ] T033 [P] [US2] End-to-end test asserting tool window state is restored after a restart in `tests/e2e/rail-persistence.spec.ts` (FR-011, SC-005)
- [ ] T034 [P] [US2] End-to-end test capturing a screenshot per rail state — each destination active, and collapsed — to `reports/screenshots/${os}/` for review, in `tests/e2e/rail-states.spec.ts`

### Implementation for User Story 2

- [ ] T035 [US2] Wire destination selection through the typed IPC wrapper in `src/lib/chrome/ActivityRail.svelte` (FR-004)
- [ ] T036 [US2] Implement collapse toggling and width retention in `src/lib/chrome/ToolWindow.svelte` (FR-006)
- [ ] T037 [US2] Implement keyboard navigation across rail destinations in `src/lib/chrome/ActivityRail.svelte` (FR-007, depends on T035)
- [ ] T038 [US2] Repair a dangling active destination on load by falling back to the first available one in `src-tauri/src/domain/rail.rs`
- [ ] T039 [US2] Render the active destination's name in the tool window header in `src/lib/chrome/ToolWindow.svelte` (FR-003)

**Checkpoint**: The rail navigates, persists and is fully keyboard-operable

---

## Phase 5: User Story 3 - Prove fidelity rather than argue it (Priority: P2)

**Goal**: One command compares the rendered shell against an approved baseline and reports what moved.

**Independent Test**: Run against an unmodified build and confirm it passes. Alter a dimension, run again, confirm it fails and names the surface.

### Baseline provenance — settle before building the gate

The baseline is a *fixture* the self-tests need, not an output of the gate. It is therefore
derived, verified and committed before any test references it. The self-tests then precede
the gate's implementation, which is ordinary tests-first ordering and not a contradiction of
the above: a test may be written before the code it exercises, but not before the data it
reads.

- [ ] T040 [US3] Derive the baseline's expected surface geometry from the prototype itself, not from our build, and record the extraction in `tools/gate-fidelity/reference/derivation.md` (SC-001). Capturing the baseline from our own output would make the gate enforce self-consistency rather than fidelity: it would lock in whatever we happened to implement, including any drift, and thereafter catch only *future* drift
- [ ] T041 [US3] Verify the implementation against the derived geometry and reconcile any difference before the baseline is committed, recording the outcome in `tools/gate-fidelity/reference/derivation.md`
- [ ] T042 [US3] Commit the verified baseline in `tools/gate-fidelity/reference/` (depends on T041)

### Tests for User Story 3

- [ ] T043 [P] [US3] Self-test asserting the gate passes on an unmodified build and fails on an altered dimension, injecting the alteration through a fixture rather than editing tracked source, in `tools/gate-fidelity/gate.test.mjs` (SC-006, depends on T042)
- [ ] T044 [P] [US3] Self-test asserting the gate errors rather than passing when the baseline is absent, unreadable, or captured at a different reference size, in `tools/gate-fidelity/gate.test.mjs` (FR-015)

### Implementation for User Story 3

- [ ] T045 [US3] Implement surface geometry measurement at the reference size in `tools/gate-fidelity/compare.mjs` (FR-013)
- [ ] T046 [US3] Implement pixel comparison against the baseline image in `tools/gate-fidelity/compare.mjs` (FR-013, depends on T045)
- [ ] T047 [US3] Implement verdict output naming each differing surface and writing a difference image in `tools/gate-fidelity/compare.mjs` (FR-014, SC-007, depends on T046)
- [ ] T048 [US3] Implement the separate baseline update command in `tools/gate-fidelity/update.mjs` (FR-016)

**Checkpoint**: Fidelity is machine-checked, and the gate has been seen to fail

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T049 Assert every end-to-end screenshot contains a rendered window rather than a blank display, in `tests/e2e/helpers.ts`. **Cross-feature**: this file is shared, so the assertion applies retroactively to F000's 34 tests; re-run F000's end-to-end suite as part of completing this task. A blank capture means the window was never shown — in F000 that was a real defect every other gate passed, and it was found by a human looking at an image rather than by any check
- [ ] T050 [P] Extend the interaction budget measurement to assert destination switching stays under 100 ms in `tests/perf/budget.spec.ts` (SC-008)
- [ ] T051 Reduce end-to-end runtime by sharing one application launch across specs that do not need a restart, in `tests/e2e/wdio.conf.ts`. **Cross-feature**: this changes the harness every feature's specs run under; F000's suite must still pass afterwards. This feature adds nine spec files to a suite already taking 16 minutes across roughly 30 relaunches
- [ ] T052 [P] Document the chrome components, the screenshot convention and the gate's operation in `docs/app-shell.md`
- [ ] T053 Run the full quickstart validation and record results in `specs/002-shell-chrome-fidelity/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies
- **Foundational (Phase 2)**: Depends on Setup — **blocks all user stories**
- **US1 (Phase 3)**: Depends on Foundational
- **US2 (Phase 4)**: Depends on Foundational and on US1's surfaces existing
- **US3 (Phase 5)**: Depends on US1; it measures what US1 builds
- **Polish (Phase 6)**: Depends on the stories it touches

### User Story Dependencies

- **US1 (P1)**: After Foundational. No dependency on other stories.
- **US2 (P1)**: Needs US1's rail and tool window to exist before navigation between them means
  anything. Independently *testable* once those surfaces render, not independently completable.
- **US3 (P2)**: Needs US1. It measures surfaces; with none built there is nothing to compare.
  Does **not** depend on US2 — navigation does not change geometry.

### Why the migration sits in Foundational

T011–T014 look like US2 work, since tool window state is what US2 persists. They are
foundational because the version change affects every session read from the moment the field
exists, including sessions belonging to features unrelated to the rail. A partially applied
migration is worse than none, so it lands complete, before any story.

### Why the baseline is committed before the self-tests

T042 commits the baseline before T043 and T044 reference it. A self-test asserting "the gate
passes on an unmodified build" cannot run without one. Tests still precede the gate's
implementation — that is deliberate — but a fixture is not implementation.

### Why baseline provenance precedes the gate

T040 and T041 come before the gate is built rather than after. A baseline captured from our
own output makes the gate enforce self-consistency instead of fidelity — it would report
success while certifying whatever drift already existed. Deriving the expected geometry from
the prototype first is what makes the gate mean what its name says.

### Within Each User Story

- Tests are written before the implementation they cover and must fail first, per Principle VII
- Domain before adapters, adapters before interface wiring
- Story complete and independently demonstrable before moving on

---

## Parallel Opportunities

- **Phase 1**: T003, T004, T005 in parallel after T001 and T002
- **Phase 2**: T009 and T010 in parallel; T013, T017, T019, T020 in parallel
- **Phase 3**: all three end-to-end tests in parallel; T024, T025 and T027 in parallel, then T026 and T028 sequence on them
- **Phase 4**: all six end-to-end tests in parallel
- **Phase 5**: both self-tests in parallel; the compare stages sequence because they share one file

### Parallel Example: User Story 1

```bash
# Tests together:
Task: "End-to-end test for chrome dimensions in tests/e2e/chrome-fidelity.spec.ts"
Task: "End-to-end test for resize behaviour in tests/e2e/chrome-resize.spec.ts"
Task: "End-to-end test for token conformance in tests/e2e/chrome-tokens.spec.ts"

# Then components in different files:
Task: "Implement chrome header in src/lib/chrome/ChromeHeader.svelte"
Task: "Implement rail button in src/lib/chrome/RailButton.svelte"
Task: "Implement tool window frame in src/lib/chrome/ToolWindow.svelte"
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1: Setup — tokens generated from the prototype
2. Phase 2: Foundational — domain and, critically, the migration
3. Phase 3: User Story 1
4. **STOP and VALIDATE**: compare against the prototype at 1200×800; review the captured screenshots
5. Demo: chrome that looks like the approved design

### Incremental Delivery

1. Setup + Foundational → an older session survives the upgrade
2. US1 → the chrome is visible and correct (MVP)
3. US2 → the rail navigates and persists
4. US3 → fidelity is machine-checked rather than eyeballed
5. Polish → blank-capture assertion, budget, runtime, documentation

### Parallel Team Strategy

Foundational is shared and mostly sequential. Afterwards US1 gates both remaining stories, so
a second developer is better used on `F001 ssh-transport-core`, which shares nothing with this
feature, than waiting on US1.

---

## Notes

- 53 tasks: 5 setup, 15 foundational, 8 (US1), 11 (US2), 9 (US3), 5 polish
- T049 and T051 modify shared harness files and must leave F000's suite passing
- [P] tasks touch different files and depend on nothing incomplete
- Test tasks are mandatory here, not optional — see the header
- Screenshots are captured automatically by the existing harness; T034 adds per-state captures
  for review and T049 asserts they are not blank
- T011–T014 and T040–T042 are the highest-risk tasks: the first group can destroy user data,
  the second can make the gate certify the wrong thing
- Commit after each task or logical group
