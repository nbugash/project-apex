# Feature Specification: Application Shell

**Feature Branch**: _none — no branch extension installed; spec directory is `specs/001-app-shell`_

**Created**: 2026-09-21

**Status**: Draft — amended 2026-09-21 for dark-only appearance under Constitution Principle I

**Input**: User description: "F000" — resolved through the feature map sequence gate to
`F000 app-shell`, whose pending subfeatures define this scope: application scaffold with
scoped capabilities, window and dockable panel layout with tab management, an asynchronous
bridge between interface and core, a status bar carrying connection and workspace state, and
the approved visual appearance.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Open the application and arrange a working layout (Priority: P1)

A developer launches the application for the first time. A window opens immediately with a
sensible default arrangement: a main area where documents will appear, a side region for
navigating a project, and a lower region for output. They drag the divider between the side
region and the main area to widen it, hide the lower region because they do not need it yet,
and quit. When they reopen the application, their arrangement is exactly as they left it.

**Why this priority**: This is the product's entire visible surface. Nothing else in the
product can be seen, used, or demonstrated until a window exists and holds its shape. It is
also the only part of the system that works with no network and no remote machine, which is
why it can be built and validated first.

**Independent Test**: Fully testable by launching the application, rearranging regions,
quitting, and relaunching. Delivers a usable, persistent workspace frame with no dependency
on any other feature.

**Acceptance Scenarios**:

1. **Given** the application has never been run, **When** the user launches it, **Then** a
   window opens with a default arrangement and requires no configuration to be usable.
2. **Given** the application is open, **When** the user resizes a region or hides and shows
   it, **Then** the change applies immediately and the arrangement remains stable during and
   after the interaction.
3. **Given** the user has customised the arrangement, **When** they quit and relaunch,
   **Then** the arrangement, window size and window position are restored as they were.
4. **Given** saved arrangement data is missing or unreadable, **When** the user launches the
   application, **Then** it opens with the default arrangement rather than failing to start.

---

### User Story 2 - Work with several documents at once (Priority: P1)

A developer has several files open. They switch between them by selecting tabs, reorder tabs
by dragging, and close ones they no longer need. On relaunch, the same documents are open and
the same one is focused.

**Why this priority**: Equal to Story 1 because a single-document frame is not a usable
development environment. Together the two form the minimum shell a developer can actually
work in.

**Independent Test**: Testable by opening several placeholder documents, switching, reordering
and closing them, then relaunching to confirm restoration. No dependency on real file content.

**Acceptance Scenarios**:

1. **Given** several documents are open, **When** the user selects a tab, **Then** that
   document becomes active and the previously active one remains open.
2. **Given** several documents are open, **When** the user drags a tab to a new position,
   **Then** the order changes and persists.
3. **Given** more tabs are open than fit the available width, **When** the user looks for a
   tab that is not visible, **Then** they can reach it without resizing the window.
4. **Given** documents are open, **When** the user quits and relaunches, **Then** the same
   documents are open and the previously focused one is focused.

---

### User Story 3 - Know the state of the session at a glance (Priority: P2)

A developer glances at the edge of the window to confirm which project they are working in and
whether the application is connected to its remote engine. When the connection drops, the
indicator changes without any action on their part, and they can tell the difference without
reading closely.

**Why this priority**: Below the frame itself because the frame must exist to hold it, but
above appearance because a developer who cannot tell whether they are connected will lose
work or waste time. It also becomes the surface every later feature reports state through.

**Independent Test**: Testable by driving state changes from a stub source and confirming the
indicator reflects each one. Requires no real connection.

**Acceptance Scenarios**:

1. **Given** the application is open, **When** the user looks at the status bar, **Then** the
   active workspace name and current connection state are both visible.
2. **Given** the application is connected, **When** the connection is lost, **Then** the
   indicator changes state within 5 seconds and without user action.
3. **Given** the user has a colour vision deficiency, **When** the connection state changes,
   **Then** the change is distinguishable by more than colour alone.
4. **Given** a workspace has a very long name, **When** it is shown in the status bar,
   **Then** the area does not expand, overlap adjacent content, or push it out of view.

---

### User Story 4 - Work in the approved appearance, consistently (Priority: P3)

A developer opens the application in a dim room. Every surface — panels, dividers, the status
area, dialogs, scrollbars, controls — renders in the approved dark appearance from the first
frame. Nothing flashes bright during launch, no element falls back to platform styling, and
the appearance does not change when the operating system switches between light and dark.

**Why this priority**: This is conformance to a design that already carries stakeholder
sign-off, not a user preference to be configured. It is last because the functional frame must
exist before it can be styled, and a partly-styled frame is still usable for development work.

**Independent Test**: Launch with the operating system set to light, and again set to dark,
confirming the appearance is identical in both cases and that no bright or unstyled frame
appears during startup. Inspect every surface for elements rendering in platform
defaults.

**Acceptance Scenarios**:

1. **Given** the operating system is set to light appearance, **When** the application
   launches, **Then** the interface renders in the approved dark appearance.
2. **Given** the application is open, **When** the operating system switches between light and
   dark appearance, **Then** the interface does not change.
3. **Given** the application is launching, **When** the window first becomes visible, **Then**
   no frame renders in a light or unstyled state before the approved appearance applies.
4. **Given** any panel, dialog or control is displayed, **When** it is inspected, **Then** its
   colour, typography, spacing and focus treatment come from the approved design rather than
   from platform defaults.

---

### Edge Cases

- **Saved layout is corrupt, truncated, or from an older version.** The application opens with
  defaults rather than failing; the unusable data is discarded, not repeatedly re-read.
- **A region is dragged to zero or near-zero size.** A minimum usable size is enforced so a
  region cannot be lost in a way the user cannot undo.
- **The window was last positioned on a display that is no longer attached.** The window is
  brought back onto an available display rather than opening off-screen.
- **Display resolution or scaling changes while running**, including moving between displays of
  different densities. Layout adapts without restart and without blurred rendering.
- **The window is made very small.** Regions degrade gracefully to a usable minimum instead of
  overlapping or clipping controls.
- **Background work runs while the user interacts.** Interaction continues smoothly; no
  operation blocks the interface.
- **Many documents are open at once.** Tab handling remains responsive and every tab remains
  reachable.
- **The user quits while work is in progress.** The application shuts down without leaving
  orphaned background processes.
- **The operating system is set to light appearance.** The application stays dark. It does not
  follow the system setting and offers no control to change it.
- **A required surface has no coverage in the approved design.** Construction of that surface
  stops until the designer resolves the gap; it is not improvised from adjacent styles.
- **Startup work fails or hangs before the window is shown.** The window is still shown, in
  whatever state it reached, rather than the application remaining invisible. A shell with a
  stale status bar is a defect; a shell nobody can see is unusable.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The application MUST launch to an interactive window with no prior configuration.
- **FR-002**: The application MUST present a default layout containing a primary document area,
  a navigation region, and an output region.
- **FR-003**: Users MUST be able to resize, hide and show regions within the window. Regions
  occupy fixed positions; moving a region to a different edge is out of scope, consistent with
  the exclusion of detachable panels in Assumptions.
- **FR-004**: The application MUST enforce a minimum usable size for every region so that no
  region can be rendered unrecoverable by resizing.
- **FR-005**: The application MUST support multiple concurrently open documents presented as
  selectable tabs, including reordering and closing.
- **FR-006**: The application MUST keep every open tab reachable when more tabs exist than fit
  the available width.
- **FR-007**: The application MUST persist window geometry, region arrangement, open documents,
  and the focused document across restarts, and restore them on launch.
- **FR-008**: The application MUST start with the default layout when persisted state is
  absent, unreadable, or incompatible, and MUST NOT fail to launch because of it.
- **FR-009**: The application MUST constrain its window to a currently attached display on
  launch.
- **FR-010**: The application MUST display a persistent status bar showing the active
  workspace and the current connection state.
- **FR-011**: The status bar MUST reflect connection state changes without user action, within
  5 seconds of the change occurring.
- **FR-012**: Connection state MUST be distinguishable by more than colour alone.
- **FR-013**: The status bar MUST handle overlong content without expanding, overlapping, or
  displacing adjacent content.
- **FR-014**: The interface MUST remain responsive to input while background work is in
  progress; no background operation may block interaction.
- **FR-015**: The application MUST render in the approved dark appearance on every surface.
  No light appearance is provided.
- **FR-016**: The application MUST NOT change appearance in response to the operating system's
  light or dark setting.
- **FR-017**: The application MUST request only the operating system capabilities it requires,
  and MUST NOT enable capabilities it does not use.
- **FR-018**: Region focus and tab navigation MUST be operable by keyboard.
- **FR-019**: The application MUST shut down without leaving orphaned background processes.
- **FR-020**: The application MUST NOT display a light or unstyled frame at any point during
  launch.
- **FR-021**: Every visible surface MUST take its colour, typography, spacing, radius and
  interaction states from the approved design. Default styling supplied by the platform,
  including the default keyboard focus indicator, is a defect.
- **FR-022**: Where the approved design does not cover a surface the feature requires, the gap
  MUST be resolved with the designer and the resolution recorded before that surface is
  built.
- **FR-023**: When persisted state cannot be written, the application MUST continue operating
  and MUST NOT block or reverse the interaction that triggered the write. The failure MUST be
  indicated to the user without a modal interruption.
- **FR-024** (qualifies FR-020): Nothing that can fail, hang, or depend on the window already
  being visible may block the point at which the window is shown. The readiness signal MUST be
  sent as early as correctness allows, and anything optional — event subscriptions, background
  wiring, telemetry — MUST happen after it.

  FR-020 is satisfied by keeping the window hidden until the interface reports readiness,
  which turns every pre-signal failure into total invisibility with nothing reported anywhere
  a user can see. During implementation this produced three distinct defects, including a
  deadlock where the signal waited on an animation frame that a hidden window never produces.
  Stated separately so it is not rediscovered by each implementer.

### Key Entities

- **Workspace session**: The project context a user is working in. Has a display name and a
  location type. At this stage its content is supplied by later features; the shell only
  displays and persists the reference.
- **Layout**: The arrangement of regions within the window — their sizes, visibility and
  positions — plus window geometry. Persisted per user, restored on launch.
- **Open document reference**: A pointer to something displayed in the document area, with an
  order within the tab strip and a focused flag. The shell holds the reference and ordering,
  not the content.
- **Connection state**: The current relationship to the remote engine, expressed as a small set
  of distinct states. Produced elsewhere; the shell renders it.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can open the application and reach an interactive window in under 2
  seconds on reference hardware (see Assumptions).
- **SC-002**: Layout, window geometry, open documents and focus are restored identically after
  a normal quit and relaunch in 100% of trials.
- **SC-003**: The application launches successfully in 100% of trials where persisted state has
  been deliberately corrupted, truncated, or removed.
- **SC-004**: No interaction — resizing a region, switching a tab, dragging a divider — produces
  a visible stall longer than 100 ms.
- **SC-005**: The interface accepts and responds to input during background work in 100% of
  test scenarios; there is no state in which the window stops responding.
- **SC-006**: Connection state is legible without interaction: the indicator is visible in the
  status bar at every supported window size without scrolling, hovering or opening a menu, and
  carries both an icon and a text label naming the state.
- **SC-007**: Connection state remains correctly distinguishable when the display is rendered in
  greyscale.
- **SC-008**: Every primary layout and tab action is reachable by keyboard alone.
- **SC-009**: The window opens fully on-screen in 100% of trials where the previously used
  display is unavailable at launch.
- **SC-010**: No orphaned background process remains after quit in 100% of trials.
- **SC-011**: No light or unstyled frame is observable during launch in 100% of trials, with
  the operating system set to light appearance and to dark appearance.
- **SC-012**: Zero surfaces render in platform default styling, verified across every panel,
  dialog and control the feature introduces.

## Assumptions

- **One main window per running application in this increment.** Multiple simultaneous project
  windows are a plausible later want but multiply layout persistence and state ownership. Not
  included here; nothing in this specification prevents adding it later.
- **Regions dock and resize; they do not detach into floating windows.** Detachable panels are a
  recognisable convenience but a substantial addition, and the value of this increment does not
  depend on them.
- **One appearance: the approved dark design.** The signed-off design system is a dark
  interface and defines no light variant, and Constitution Principle I makes it authoritative.
  A light appearance would require new design work and fresh stakeholder sign-off, not an
  implementation decision. User-defined or importable themes are likewise out of scope.
- **Connection state and workspace identity are supplied by a stub during this increment.** The
  transport and workspace features that produce them for real are separate and may not exist
  yet, so the shell must be demonstrable and testable against simulated state.
- **The document area displays placeholder content in this increment.** Real document display
  belongs to the editor feature; this specification covers the frame that holds it.
- **Target platforms are macOS and Linux.** Windows is out of scope for the product.
- **A single user per installation**, with preferences stored locally. No profile sync, no
  shared or multi-user state.
- **Accessibility baseline is keyboard operability and non-colour-dependent state.** Full screen
  reader support is not specified here and is not precluded.
- **The unfamiliar-developer usability trial is a validation activity, not an acceptance
  gate.** Whether a new user identifies connection state unaided is worth measuring, but it
  needs recruited subjects and cannot run in continuous integration. SC-006 now states the
  structural properties that make it likely and that a test can assert; the trial itself
  belongs to product validation after the feature ships.
- **Reference hardware for timing criteria** is a developer laptop from the last four years with
  a solid-state drive and at least 8 GB of memory. Timing outcomes are measured on a machine
  meeting or exceeding that, from a cold start with no other application under load.
