# Feature Specification: Shell Chrome Fidelity

**Feature Branch**: _none — no branch extension installed; spec directory is `specs/002-shell-chrome-fidelity`_

**Created**: 2026-09-21

**Status**: Draft

**Input**: User description: "F018" — resolved through the feature map sequence gate to
`F018 shell-chrome-fidelity`, whose pending subfeatures define this scope: an activity icon
rail at the prototype's width navigating between tool windows, a window chrome header row,
tool window header and tree row rhythm, the prototype's layout dimensions carried as design
tokens, and a repeatable visual fidelity comparison against the approved prototype.

## On the source of values

This specification names **which surfaces must match** and **how conformance is judged**. It
does not restate the prototype's measurements, colours or spacing.

That is deliberate. The signed-off prototype is the authority for every value, and copying
those values into prose would create a second source of truth that drifts from it on the
first design change — the precise failure the constitution's design fidelity principle
exists to prevent. Where this document needs to refer to a dimension, it refers to the
prototype.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Recognise the application as the approved design (Priority: P1)

A developer who has seen the approved prototype opens the application. The window chrome,
the activity rail down the left edge, and the tool window all look like the prototype they
signed off — same proportions, same rhythm, same weight. Nothing reads as an approximation
built from a screenshot.

**Why this priority**: This is the feature. The shell currently presents three generic
regions; the prototype specifies particular chrome, and the gap between them is what a
stakeholder notices first and trusts least.

**Independent Test**: Place the running application beside the prototype at the same window
size and compare the three surfaces. Differences in position, size or weight are defects.

**Acceptance Scenarios**:

1. **Given** the application is open at the reference size (see Assumptions), **When** it is
   compared against the prototype, **Then** the chrome header, activity rail and tool window
   occupy the same positions and dimensions within the comparison tolerance.
2. **Given** any of those surfaces, **When** its colour, spacing, typography or border
   treatment is inspected, **Then** each value resolves to the design system rather than a
   literal chosen during implementation.
3. **Given** the window is resized, **When** the layout reflows, **Then** the fixed-width
   surfaces keep the prototype's widths and only the flexible regions absorb the change.

---

### User Story 2 - Move between tool windows from the rail (Priority: P1)

A developer clicks an icon in the activity rail. The tool window beside it switches to that
view, and the rail shows which one is active. They can reach every rail destination by
keyboard, and the active one is identifiable without relying on colour.

**Why this priority**: Equal to Story 1 because a rail that looks right but does nothing is
a picture, not a feature. The prototype's rail carries navigation, and the shell has no
mechanism for switching tool windows at all.

**Independent Test**: Select each rail destination in turn and confirm the tool window
changes and the active state moves with it. Repeat using only the keyboard.

**Acceptance Scenarios**:

1. **Given** the rail is visible, **When** a destination is selected, **Then** the tool
   window shows that destination and the rail marks it active.
2. **Given** a destination is active, **When** it is selected again, **Then** the tool window
   collapses, and selecting it once more restores it at its previous width.
3. **Given** focus is in the rail, **When** the user navigates by keyboard alone, **Then**
   every destination is reachable and activatable.
4. **Given** the display is rendered in greyscale, **When** a destination is active, **Then**
   it remains distinguishable from the inactive ones.

---

### User Story 3 - Prove fidelity rather than argue it (Priority: P2)

A developer changes a component and wants to know whether they have drifted from the
approved design. They run one command. It compares the rendered shell against an approved
baseline and reports what moved, if anything.

**Why this priority**: Below the surfaces themselves because there must be something to
compare before comparing is useful. Above nothing, because without it fidelity decays
silently: token linting catches a wrong *value* but says nothing about a surface that is
forty pixels too wide.

**Independent Test**: Run the comparison against an unmodified build and confirm it passes.
Alter a dimension, run it again, and confirm it fails and names what changed.

**Acceptance Scenarios**:

1. **Given** an unmodified build, **When** the comparison runs, **Then** it passes.
2. **Given** a surface whose dimension has been changed, **When** the comparison runs,
   **Then** it fails and identifies the surface that differs.
3. **Given** the comparison fails, **When** the developer inspects the result, **Then** an
   artifact showing the difference is available.
4. **Given** the approved design itself changes, **When** the baseline is deliberately
   updated, **Then** the update is an explicit, reviewable act rather than an automatic one.

---

### Edge Cases

- **The window is narrower than the fixed surfaces allow.** The rail and tool window hold
  their widths down to the minimum window size; below that the flexible regions have already
  reached their own minimums and the window cannot shrink further.
- **The tool window is collapsed.** The rail remains visible and functional; collapsing a
  panel must not remove the means of restoring it.
- **A rail destination has no tool window built yet.** It is either absent from the rail or
  visibly unavailable — never present and silently inert.
- **The prototype specifies a surface this feature does not build.** Construction stops and
  the gap goes to the designer, rather than being approximated from an adjacent surface.
- **The baseline is missing or unreadable.** The comparison reports that it cannot run rather
  than passing by default.
- **Rendering differs immaterially between machines.** The comparison tolerates differences
  within the stated tolerance and fails outside it.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The application MUST present a window chrome header carrying the product mark
  and the active project, matching the prototype's layout and dimensions.
- **FR-002**: The application MUST present an activity rail along the window edge at the
  prototype's width, listing the available tool window destinations.
- **FR-003**: The application MUST present a tool window with a header row, at the
  prototype's width and rhythm.
- **FR-004**: Selecting a rail destination MUST switch the tool window to that destination.
- **FR-005**: The rail MUST indicate which destination is active, by more than colour alone.
- **FR-006**: Selecting the active destination MUST collapse the tool window, and selecting
  it again MUST restore it at its previous width.
- **FR-007**: Every rail destination MUST be reachable and activatable by keyboard.
- **FR-008**: A rail destination whose tool window does not exist MUST be visibly unavailable
  rather than present and inert.
- **FR-009**: Every dimension the prototype specifies for these surfaces MUST be expressed as
  a design token, not as a literal at the point of use.
- **FR-010**: The fixed-width surfaces MUST retain their widths as the window resizes; only
  the flexible regions absorb the change.
- **FR-011**: The collapsed or expanded state of the tool window, and the active destination,
  MUST persist across restarts.
- **FR-012**: A single command MUST compare the rendered shell against an approved baseline
  and report whether they differ.
- **FR-013**: The comparison MUST fail when a surface's position or dimension differs from
  the baseline by more than the position tolerance, or when the proportion of differing
  pixels exceeds the area tolerance. Both are defined in Assumptions.
- **FR-014**: A failed comparison MUST produce an artifact showing what differs.
- **FR-015**: The comparison MUST report an error, rather than passing, when the baseline is
  absent or unreadable.
- **FR-016**: Updating the approved baseline MUST be an explicit action, never a side effect
  of running the comparison.

### Key Entities

- **Rail destination**: One entry in the activity rail. Has an identity, a label, an icon, an
  availability state, and a relationship to the tool window it opens.
- **Tool window state**: Which destination is active, whether the panel is collapsed, and its
  width when expanded. Persisted with the rest of the session.
- **Fidelity baseline**: The approved rendering the comparison judges against, plus the
  tolerance within which differences are ignored. Updated deliberately, never automatically.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: At the reference size, the chrome header, activity rail and tool window each
  match the prototype's position and dimension to within the position tolerance.
- **SC-002**: Zero dimensions for these surfaces appear as literals in the application;
  every one resolves through a design token.
- **SC-003**: Every rail destination is reachable and activatable using only the keyboard.
- **SC-004**: The active destination remains identifiable when the display is rendered in
  greyscale.
- **SC-005**: Tool window state — active destination, collapsed or expanded, and width —
  is restored identically after a normal quit and relaunch in 100% of trials.
- **SC-006**: The fidelity comparison passes on an unmodified build and fails on a
  deliberately altered dimension, in 100% of trials.
- **SC-007**: A failed comparison names the differing surface and produces a difference
  artifact in 100% of failures.
- **SC-008**: Switching tool window destinations produces no visible stall longer than
  100 ms.

## Assumptions

- **The prototype is the authority for every value.** Measurements, colours, spacing and
  typography come from it rather than from this document. Where the two could disagree, the
  prototype wins and this document is wrong.
- **The rail's destinations are those the prototype shows.** Destinations belonging to
  features not yet built are marked unavailable rather than omitted, so the rail's proportions
  match the prototype from the outset.
- **Tool window content is out of scope.** This feature builds the frame, its header and its
  rhythm. What each destination displays belongs to the feature that owns it — the file tree
  to the workspace feature, version control to git integration, and so on.
- **One tool window region, on one edge.** The prototype shows a single docked panel. Multiple
  simultaneous tool windows, or docking to other edges, are not included.
- **The comparison runs where the end-to-end suite runs.** It needs a rendered window, so it
  is subject to the same platform constraint recorded for end-to-end coverage: it runs on
  Linux, and the platform without a driver gets the existing smoke check instead.
- **Rendering varies slightly between machines**, so the comparison is tolerance-based rather
  than exact. An exact pixel match would fail on font rasterisation differences nobody can see,
  and a gate that cries wolf gets switched off.

  **Reference size**: 1200 by 800, the application's default window size. The comparison is
  performed at this size so that a baseline is reproducible.

  **Position tolerance**: 2 device pixels. Covers sub-pixel rounding at standard density while
  remaining far below the smallest misplacement a viewer notices.

  **Area tolerance**: 0.5% of compared pixels may differ. Absorbs antialiasing variance across
  machines; a surface of the wrong size moves far more than this.

  These are engineering thresholds, not design decisions. If the comparison proves too noisy
  or too permissive in practice, they are the knobs to turn, and turning them is a recorded
  change rather than an ad-hoc adjustment.

- **The unavailable rail state is this feature's invention, not the prototype's.** Every
  destination in the prototype works, so it specifies no unavailable treatment, while FR-008
  requires one. It is rendered at reduced opacity — deliberately not a colour change, so the
  state survives greyscale (SC-004) — and the choice is flagged here for the designer under
  FR-022 rather than presented as approved design.

## Open Items

_None outstanding._

### Resolved

- **[RESOLVED: PANEL-STATE] The left panel had two persisted descriptions of its geometry.**
  F000 modelled the left panel as a generic region, `layout.navigation`, carrying `visible`
  and `extent`. This feature gave that position its real identity — the prototype's tool
  window — and `ToolWindowState` carried `collapsed` and `width` for the same surface.

  **Resolution**: `layout.navigation` is retired. The tool window owns its own geometry, and
  `Layout` keeps only the generic regions that remain (`output`, `document_area`).

  Two options were weighed. The alternative was to shrink `ToolWindowState` to
  `active_destination_id` and leave the panel's geometry in `Layout` alongside the other
  regions. That was the initial recommendation, on the grounds that retiring
  `layout.navigation` would need a new schema version. The premise was wrong: schema
  version 2 had never shipped, so its shape could be redefined rather than superseded, and
  the migration cost that favoured the alternative did not exist.

  With cost equal, FR-006 decided it. Collapse-on-reselect with width retention is a single
  invariant over a single type when the tool window owns its geometry; splitting the state
  across `Layout` and `ToolWindowState` would have made one selection mutate two types, with
  the invariant spread between them, and would have kept a region named `navigation` for a
  surface that is not navigation.

  **Migration**: `Layout` reads a legacy `navigation` region from older files, folds its
  extent and visibility into the tool window on load, and never writes it back — so an
  existing session keeps the panel width its user chose, and the field disappears on the
  next save. The fold is conditional; an unconditional one would reset the panel for every
  file written after the upgrade.
