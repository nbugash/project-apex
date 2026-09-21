---
description: "Task list for the Application Shell feature"
---

# Tasks: Application Shell

**Input**: Design documents from `/specs/001-app-shell/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, architecture.md, design.md

**Tests**: Included and **not optional**. Constitution Principle VII requires unit,
integration and end-to-end coverage before a feature is complete, with a recorded
justification for any omitted level. The one omission here — end-to-end on macOS — is
justified in plan.md Complexity Tracking.

**Organization**: Grouped by user story. Stories US1 and US2 are both P1; US1 leads because a
frame without tabs is demonstrable, while tabs without a frame are not.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on incomplete work)
- **[Story]**: US1–US4, mapping to the user stories in spec.md

## Path Conventions

Two source trees, per the Structure Decision in plan.md: `src-tauri/` for the Rust core,
`src/` for the Svelte interface layer, `tests/` at repository root for interface-layer and
end-to-end suites.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project skeleton, toolchain, and the design-system pipeline every later phase depends on

- [X] T001 Create the two source trees and directory skeleton per plan.md Structure Decision
- [X] T002 Initialize the Rust core with Tauri v2, tokio, serde and serde_json in `src-tauri/Cargo.toml`
- [X] T003 [P] Initialize Svelte 5 + Vite + TypeScript in `package.json`, `vite.config.ts` and `tsconfig.json`
- [X] T004 [P] Configure rustfmt and clippy in `src-tauri/rustfmt.toml` and `src-tauri/clippy.toml`
- [ ] T005 [P] Configure ESLint and Prettier for the interface layer in `eslint.config.js`
- [X] T006 Implement the `ds:sync` script copying the signed-off design system from `mockups/` into `src/lib/ds/` in `scripts/ds-sync.mjs`, failing the build when the source is absent rather than degrading to unstyled output
- [X] T007 Port the design adherence lint from its React plugin configuration to the Svelte toolchain in `eslint.config.js`, preserving every rule it expresses (no raw hex, no raw pixel values, no hard-coded font families)
- [X] T008 [P] Wire `lint:ds` and `perf:budget` as required checks in `.github/workflows/ci.yml`
- [X] T009 [P] Define the least-privilege capability set in `src-tauri/capabilities/default.json`
- [X] T010 [P] Configure structured file logging to the application data directory in `src-tauri/src/logging.rs`

**Checkpoint**: Toolchain builds, design system syncs, adherence lint runs and passes on an empty tree

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The core the shell is made of. Every user story renders through this window, reads this session state and crosses this bridge, so none can start until it exists.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

### Domain types

- [X] T011 [P] Define `ShellError` and its variants in `src-tauri/src/application/error.rs` per contracts/shell-commands.md
- [X] T012 [P] Define `RegionId`, `RegionState` and `Layout` with minimum-extent rules in `src-tauri/src/domain/layout.rs`
- [X] T013 [P] Define `WindowGeometry` and the display-intersection rule in `src-tauri/src/domain/geometry.rs`
- [X] T014 [P] Define `DocumentId`, `OpenDocumentReference`, `WorkspaceReference`, `PersistedSession` and `SessionSnapshot` in `src-tauri/src/domain/session.rs`
- [X] T015 [P] Define `ConnectionState` and its permitted transitions in `src-tauri/src/domain/connection.rs`

### Domain unit tests

- [X] T016 [P] Unit tests for region extent clamping and the document-area-not-hideable rule in `src-tauri/src/domain/layout.rs`
- [X] T017 [P] Unit tests for title-bar-within-working-area intersection, including the detached-display case, in `src-tauri/src/domain/geometry.rs`
- [X] T018 [P] Unit tests for document ordering contiguity, re-packing on close, and focus reassignment in `src-tauri/src/domain/session.rs`

### Ports

- [X] T019 [P] Define the `SessionStore` port in `src-tauri/src/application/ports/session_store.rs`
- [X] T020 [P] Define the `ConnectionStatusSource` port in `src-tauri/src/application/ports/connection.rs`

### Adapters and use cases

- [X] T021 Implement `JsonFileSessionStore` in `src-tauri/src/adapters/outbound/json_session_store.rs`, treating unreadable and invalid content as absence rather than error
- [X] T022 Integration test for the session store against real files — round trip, absent file, malformed JSON, truncated file, future `schema_version`, dangling `focused_document_id` — in `src-tauri/tests/session_store.rs`
- [X] T023 Implement the `RestoreSession` use case in `src-tauri/src/application/use_cases/restore_session.rs` (depends on T019, T021)
- [ ] T024 Implement the `PersistSession` use case with debounced off-path writes in `src-tauri/src/application/use_cases/persist_session.rs` (depends on T019)
- [X] T025 Implement `WindowController` with `create_hidden`, `apply` and idempotent `mark_ready` in `src-tauri/src/window/controller.rs`
- [X] T026 Integration test asserting restored geometry is constrained to an attached display in `src-tauri/tests/display_geometry.rs`
- [X] T027 Implement the `TauriCommandAdapter` with boundary validation of every argument in `src-tauri/src/adapters/inbound/tauri_commands.rs` (depends on T011, T023, T024, T025)
- [X] T028 Implement the composition root binding adapters to use cases in `src-tauri/src/composition.rs`
- [X] T029 Wire the entry point to create the window hidden and restore session before showing in `src-tauri/src/main.rs` (depends on T025, T028)
- [X] T030 [P] Implement the typed IPC wrapper over the command surface in `src/lib/ipc.ts`
- [X] T031 Implement the `shell_ready` readiness handshake end to end, from `src/main.ts` through `src-tauri/src/adapters/inbound/tauri_commands.rs` to `WindowController::mark_ready`

**Checkpoint**: The window opens hidden, restores or defaults its session, and becomes visible on the readiness signal. User stories can now proceed in parallel.

---

## Phase 3: User Story 1 - Open the application and arrange a working layout (Priority: P1) 🎯 MVP

**Goal**: A window with three resizable, hideable regions whose arrangement survives restart and degrades to defaults when persisted state is unusable.

**Independent Test**: Launch, rearrange regions, quit, relaunch, confirm the arrangement is restored. Corrupt the state file and confirm the application still opens.

### Tests for User Story 1

- [ ] T032 [P] [US1] End-to-end test for layout, visibility and geometry restoration across restart in `tests/e2e/layout-persistence.spec.ts` (quickstart scenario 1, SC-002)
- [ ] T033 [P] [US1] End-to-end test asserting launch succeeds with corrupted, truncated, empty and dangling-reference state in `tests/e2e/corrupt-state.spec.ts` (quickstart scenario 2, SC-003)
- [ ] T034 [P] [US1] End-to-end test asserting the window opens on an attached display when the saved display is gone in `tests/e2e/display-recovery.spec.ts` (quickstart scenario 7, SC-009)
- [X] T035 [P] [US1] Unit test for splitter clamping at the minimum region extent in `tests/unit/splitter.test.ts`

### Implementation for User Story 1

- [X] T036 [P] [US1] Implement the region composition grid in `src/lib/shell/Window.svelte`
- [X] T037 [P] [US1] Implement a single dockable region with visibility handling in `src/lib/shell/Region.svelte`
- [X] T038 [US1] Implement the pointer-driven divider with minimum-extent enforcement in `src/lib/shell/Splitter.svelte` (depends on T037)
- [X] T039 [US1] Wire `layout_set_region` from the splitter and visibility toggles through `src/lib/ipc.ts` (depends on T030, T038)
- [X] T040 [US1] Subscribe to native window move and resize events and record geometry in `src-tauri/src/window/controller.rs` (depends on T024)

**Checkpoint**: User Story 1 is fully functional and independently testable. This is the MVP.

---

## Phase 4: User Story 2 - Work with several documents at once (Priority: P1)

**Goal**: Multiple open documents as reorderable, closeable tabs that restore with their order and focus.

**Independent Test**: Open several documents, reorder, close one, focus another, quit, relaunch, confirm the same set, order and focus. Open more than fit and confirm every tab remains reachable.

### Tests for User Story 2

- [ ] T041 [P] [US2] End-to-end test for tab set, order and focus restoration across restart in `tests/e2e/tab-persistence.spec.ts` (quickstart scenario 3, SC-002)
- [ ] T042 [P] [US2] End-to-end test asserting every tab stays reachable past the strip width in `tests/e2e/tab-overflow.spec.ts` (quickstart scenario 4, FR-006)
- [X] T043 [P] [US2] Unit test for tab strip ordering and focus transfer on close in `tests/unit/tab-strip.test.ts`

### Implementation for User Story 2

- [X] T044 [P] [US2] Implement the scrolling tab strip with drag-to-reorder in `src/lib/tabs/TabStrip.svelte`
- [X] T045 [P] [US2] Implement the overflow control listing every open document in `src/lib/tabs/TabOverflow.svelte`
- [X] T046 [US2] Wire `documents_open`, `documents_close`, `documents_reorder` and `documents_focus` through `src/lib/ipc.ts` (depends on T030, T044, T045)
- [X] T047 [US2] Render the focused document reference in the document area placeholder in `src/lib/shell/Window.svelte` (depends on T036)

**Checkpoint**: User Stories 1 and 2 both work independently.

---

## Phase 5: User Story 3 - Know the state of the session at a glance (Priority: P2)

**Goal**: A status area reporting workspace identity and connection state, updating without user action and readable without relying on colour.

**Independent Test**: Drive the stub through every connection transition and confirm the status area reflects each within 5 seconds. Repeat in greyscale and confirm all states remain distinguishable.

### Tests for User Story 3

- [ ] T048 [P] [US3] End-to-end test for connection state transitions appearing within 5 seconds in `tests/e2e/connection-status.spec.ts` (quickstart scenario 5, FR-011)
- [X] T049 [P] [US3] Unit test asserting each connection state carries a non-colour distinguishing attribute in `tests/unit/status-bar.test.ts` (FR-012, SC-007)
- [ ] T050 [P] [US3] End-to-end test asserting the connection indicator is visible without scrolling, hovering or opening a menu at minimum and maximum window size, and carries both icon and text label, in `tests/e2e/status-legibility.spec.ts` (SC-006)
- [X] T051 [P] [US3] Unit test for overlong workspace name truncation without layout displacement in `tests/unit/status-bar.test.ts` (FR-013)

### Implementation for User Story 3

- [X] T052 [P] [US3] Implement `StubConnectionStatusSource` with a development control in `src-tauri/src/adapters/outbound/stub_connection.rs`
- [X] T053 [US3] Implement the `ObserveConnection` use case emitting `connection:changed` in `src-tauri/src/application/use_cases/observe_connection.rs` (depends on T020, T052)
- [ ] T054 [US3] Emit `workspace:changed` once at startup and on change in `src-tauri/src/adapters/inbound/tauri_commands.rs` (depends on T027)
- [X] T055 [US3] Implement the status area with icon-plus-text state encoding and name truncation in `src/lib/statusbar/StatusBar.svelte`
- [ ] T056 [US3] Subscribe to both events and bind them to the status area in `src/lib/ipc.ts` and `src/main.ts` (depends on T030, T053, T055)
- [ ] T057 [P] [US3] Add the stub driver command used by quickstart scenario 5 in `scripts/stub-connection.mjs`

**Checkpoint**: User Stories 1, 2 and 3 all work independently.

---

## Phase 6: User Story 4 - Work in the approved appearance, consistently (Priority: P3)

**Goal**: Every surface renders in the approved dark appearance from the first painted frame, with no light or unstyled frame and no platform default styling anywhere.

**Independent Test**: Launch with the operating system set to light and then to dark, confirm identical appearance and no bright frame during startup. Confirm the adherence lint passes and no surface uses platform defaults.

### Tests for User Story 4

- [ ] T058 [P] [US4] End-to-end test capturing frames from window creation and asserting none is light or unstyled, under both OS appearance settings, in `tests/e2e/launch-appearance.spec.ts` (quickstart scenario 6, SC-011)
- [ ] T059 [P] [US4] End-to-end test asserting the interface does not change when the OS appearance setting changes in `tests/e2e/os-appearance-ignored.spec.ts` (FR-016)
- [ ] T060 [P] [US4] Automated audit asserting zero surfaces render in platform default styling in `tests/e2e/token-conformance.spec.ts` (FR-021, SC-012)

### Implementation for User Story 4

- [X] T061 [US4] Set the window background to the design system ground value and `visible: false` at creation in `src-tauri/tauri.conf.json` (research.md, "Preventing a light or unstyled first frame")
- [X] T062 [US4] Import the synced design system and define nothing locally in `src/app.css`
- [X] T063 [US4] Apply design system classes and tokens to `src/lib/shell/Window.svelte`, `src/lib/shell/Region.svelte`, `src/lib/shell/Splitter.svelte`, `src/lib/tabs/TabStrip.svelte`, `src/lib/tabs/TabOverflow.svelte` and `src/lib/statusbar/StatusBar.svelte` (depends on T036, T044, T055)
- [X] T064 [US4] Replace the platform default focus indicator with the design system focus ring on every interactive element in `src/lib/shell/Splitter.svelte`, `src/lib/tabs/TabStrip.svelte`, `src/lib/tabs/TabOverflow.svelte` and `src/lib/statusbar/StatusBar.svelte` (FR-021)
- [ ] T065 [US4] Implement the designer-gap procedure check: fail the build when a component references a token the design system does not define, in `scripts/ds-sync.mjs` (FR-022)

**Checkpoint**: All four user stories are independently functional and the interface conforms to the signed-off design.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Requirements that span stories, plus the measurements the constitution requires

- [X] T066 [P] Implement keyboard navigation across regions and tabs in `src/lib/shell/Window.svelte` and `src/lib/tabs/TabStrip.svelte` (FR-018)
- [ ] T067 [P] End-to-end test asserting every primary layout and tab action is reachable by keyboard alone in `tests/e2e/keyboard.spec.ts` (SC-008)
- [ ] T068 Implement clean shutdown with no orphaned background processes in `src-tauri/src/main.rs` (FR-019)
- [ ] T069 [P] Integration test asserting no orphaned process survives quit in `src-tauri/tests/shutdown.rs` (SC-010)
- [ ] T070 Implement the interaction budget measurement asserting launch under 2 s and no stall over 100 ms in `tests/perf/budget.spec.ts` (SC-001, SC-004, Constitution Principle V)
- [ ] T071 [P] End-to-end test asserting the interface stays responsive during background work in `tests/e2e/responsiveness.spec.ts` (SC-005)
- [ ] T072 [P] Implement the macOS smoke check — launch, await readiness, screenshot, assert clean exit — in `scripts/smoke-macos.sh`
- [X] T073 [P] Add a non-blocking indication for persistence failure in `src/lib/statusbar/StatusBar.svelte` (FR-023, contracts/shell-commands.md `PersistenceFailed`)
- [ ] T074 Run the full quickstart.md validation and record results
- [ ] T075 [P] Document the shell architecture and the port boundaries for later features in `docs/app-shell.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies
- **Foundational (Phase 2)**: Depends on Setup — **blocks all user stories**
- **User Stories (Phases 3–6)**: All depend on Foundational; independent of one another afterwards
- **Polish (Phase 7)**: Depends on the stories whose surfaces it touches

### User Story Dependencies

- **US1 (P1)**: After Foundational. No dependency on other stories. Region repositioning is out of scope per FR-003; regions resize, hide and show in fixed positions.
- **US2 (P1)**: After Foundational. Renders inside the document area from US1 but is testable against the default layout, so it does not require US1 to be complete.
- **US3 (P2)**: After Foundational. Fully independent — the status area occupies its own region.
- **US4 (P3)**: After Foundational, and — unlike the others — **not independent in completion**. T060, T063 and T064 style and audit the surfaces US1, US2 and US3 create, so US4 is independently *testable* against whatever surfaces exist at the time but cannot be *complete* before them. Sequenced last for that reason, not merely so it avoids delaying the functional frame.

### Why this phase split is foundational-heavy

Unusually for a feature of this size, Phase 2 carries 21 tasks. All four stories render in one
window, read one session state and cross one bridge, so the domain types, both ports, the
session store, the window controller and the command adapter genuinely block every story
rather than belonging to any one of them. Pushing them into US1 would make US2, US3 and US4
depend on US1 and destroy their independence.

### Within Each User Story

- Tests are written before the implementation they cover and must fail first, per Constitution Principle VII, which prefers fail-first wherever a wrong answer is expensive
- Domain before adapters, adapters before interface wiring
- Story complete and independently demonstrable before moving on

---

## Parallel Opportunities

- **Phase 1**: T003, T004, T005, T008, T009, T010 in parallel after T001 and T002
- **Phase 2**: all five domain types (T011–T015) in parallel; their unit tests (T016–T018) in parallel; both ports (T019, T020) in parallel
- **Phases 3–6**: once Foundational completes, all four stories can proceed simultaneously with sufficient staffing
- **Within each story**: all test tasks in parallel, then components in different files in parallel

### Parallel Example: User Story 1

```bash
# Tests together:
Task: "End-to-end test for layout restoration in tests/e2e/layout-persistence.spec.ts"
Task: "End-to-end test for corrupt state in tests/e2e/corrupt-state.spec.ts"
Task: "End-to-end test for display recovery in tests/e2e/display-recovery.spec.ts"
Task: "Unit test for splitter clamping in tests/unit/splitter.test.ts"

# Then components in different files:
Task: "Implement region composition in src/lib/shell/Window.svelte"
Task: "Implement a dockable region in src/lib/shell/Region.svelte"
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1: Setup
2. Phase 2: Foundational — the long pole; nothing demonstrable until it lands
3. Phase 3: User Story 1
4. **STOP and VALIDATE**: quickstart scenarios 1, 2 and 7
5. Demo: a window that holds its shape across restarts and survives a corrupted state file

### Incremental Delivery

1. Setup + Foundational → a window that opens and restores
2. US1 → arrangeable, persistent layout (MVP)
3. US2 → multiple documents
4. US3 → session state visible at a glance
5. US4 → conformance to the signed-off appearance
6. Polish → keyboard, shutdown, budget measurement, macOS smoke check

### Parallel Team Strategy

Setup and Foundational are shared and mostly sequential. Once Phase 2 completes, four
developers can take one story each with no cross-story coordination beyond US4, which styles
surfaces the others create and is therefore most efficient last.

---

## Notes

- 75 tasks total: 10 setup, 21 foundational, 9 (US1), 7 (US2), 10 (US3), 8 (US4), 10 polish
- [P] tasks touch different files and depend on nothing incomplete
- Test tasks are mandatory here, not optional — see the header
- The only omitted test level is end-to-end on macOS, justified in plan.md Complexity Tracking
- Commit after each task or logical group
- Stop at any checkpoint to validate a story independently
