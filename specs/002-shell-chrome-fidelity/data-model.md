# Data Model: Shell Chrome Fidelity

**Branch**: `002-shell-chrome-fidelity` | **Date**: 2026-09-21 | **Plan**: [plan.md](./plan.md)

Entities this feature introduces, plus the one it extends. Field-level definitions live here
and are not restated in `design.md`. The persisted shape is fixed by
[contracts/session-state-v2.schema.json](./contracts/session-state-v2.schema.json).

Entities owned by F000 — `PersistedSession`, `Layout`, `WindowGeometry`,
`OpenDocumentReference`, `ConnectionState` — are defined in
[`../001-app-shell/data-model.md`](../001-app-shell/data-model.md) and are referenced, not
duplicated.

---

## Constants

| Constant | Value | Basis |
|----------|-------|-------|
| `SCHEMA_VERSION` | 2 | Raised from 1 by the tool window fields below |
| `MIN_TOOL_WINDOW_WIDTH` | 180 px | Below this the prototype's tool window header truncates its own title, so the panel is present but unreadable |
| `FIDELITY_POSITION_TOLERANCE` | 2 device px | Spec Assumptions; covers sub-pixel rounding at standard density |
| `FIDELITY_AREA_TOLERANCE` | 0.5% of compared pixels | Spec Assumptions; absorbs antialiasing variance across machines |
| `FIDELITY_REFERENCE_SIZE` | 1200 × 800 | Spec Assumptions; the application's default window size, so a baseline is reproducible |

Layout dimensions taken from the prototype — rail width, chrome header height, tool window
width, row heights — are deliberately absent from this table. They are generated from the
prototype at build time (see [research.md](./research.md), "Where the prototype's layout
dimensions live"); listing them here would create the second source of truth that decision
exists to prevent.

---

## RailDestination

One entry in the activity rail. The set is static and derived from the prototype.

Type names shorten "activity rail" to "rail" — `RailDestination`, `RailCatalogue`, `rail.ts`.
They refer to the same surface the specification calls the activity rail.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `id` | string | yes | Stable identifier, e.g. `files`, `vcs`, `search` |
| `label` | string | yes | Accessible name and tooltip text |
| `icon` | string | yes | Icon identifier from the bundled set |
| `availability` | `"available"` \| `"unavailable"` | yes | `unavailable` when the owning feature is not built |
| `order` | integer | yes | Position in the rail; matches the prototype's order |

**Validation**

- `id` MUST be unique across destinations.
- `order` MUST form a contiguous zero-based sequence, so the rail's spacing matches the
  prototype exactly rather than approximately.
- An `unavailable` destination MUST still render in its position (FR-008 and the spec's
  Assumptions): omitting it would change the rail's proportions and therefore its fidelity.
- `availability` MUST be reflected by more than colour, so the distinction survives greyscale.

---

## ToolWindowState

Which destination is showing, whether the panel is collapsed, and how wide it is.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `active_destination_id` | string \| null | yes | `null` only when no destination has ever been selected |
| `collapsed` | boolean | yes | |
| `width` | integer | yes | ≥ `MIN_TOOL_WINDOW_WIDTH`; retained while collapsed |

**Validation**

- `active_destination_id`, when non-null, MUST match a known `RailDestination.id`. A
  destination that has been removed between versions leaves a dangling reference; that is
  repaired on load by falling back to the first available destination, not by discarding the
  session.
- `width` below the minimum is clamped on load and rejected on a live command — the same
  asymmetry F000 established for region extents, and for the same reason: a stale file should
  not cost the user their session, while a bad live command is an interface defect that should
  surface.
- `collapsed` MUST NOT imply a reset of `width`; showing a collapsed panel again restores its
  previous size.

---

## PersistedSession (extended)

Defined in [`../001-app-shell/data-model.md`](../001-app-shell/data-model.md). This feature
adds one field and raises the version.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `schema_version` | integer | yes | Now `2` |
| `tool_window` | ToolWindowState | yes | Added by this feature |

**Migration**

The load rule changes, and the change is the substance of this feature's riskiest decision
(see [research.md](./research.md), "Extending the persisted session without discarding it").

| File version | Behaviour |
|--------------|-----------|
| Equal to current | Loaded as-is |
| Below current | Missing fields filled with defaults, rewritten at the current version |
| Above current | Discarded, defaults applied — a build cannot interpret a future shape |

The previous rule discarded anything that was not exactly current. Retaining that would mean
every existing user loses their window geometry, layout and open tabs on upgrade, purely
because a panel was added. FR-008 of the shell requires invalid state not to prevent launch;
it was never intended to discard *valid older* state.

---

## FidelityBaseline

The approved rendering the comparison judges against. A committed golden file, updated only
by an explicit command.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `reference_size` | `{ width, height }` | yes | Must equal `FIDELITY_REFERENCE_SIZE` |
| `surfaces` | SurfaceGeometry[] | yes | One per measured surface |
| `image` | file reference | yes | The approved rendering, stored beside the geometry |
| `captured_at` | timestamp | yes | For provenance in review |

**Validation**

- A baseline whose `reference_size` differs from the current constant MUST cause the
  comparison to report an error rather than pass; comparing at a different size is
  meaningless.
- An absent or unreadable baseline MUST be an error, never a pass (FR-015). A gate that passes
  when it cannot run is worse than no gate.

---

## SurfaceGeometry

One measured surface within the baseline.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `name` | string | yes | e.g. `chrome-header`, `activity-rail`, `tool-window` |
| `x`, `y` | integer | yes | Position relative to the window |
| `width`, `height` | integer | yes | |

**Validation**

- `name` MUST be unique, and MUST identify the surface in failure output so a reader knows
  what moved without opening an image.

---

## RailNavigation

Not persisted; the runtime lifecycle of selecting a destination.

```text
        ┌───────────────── select a different destination ─────────────┐
        ▼                                                              │
   [ Collapsed ] ──select active──► [ Showing destination ] ──select active──► [ Collapsed ]
        │                                    │
        └────── select any destination ──────┘
```

**Validation**

- Selecting an `unavailable` destination MUST NOT change the active destination or the
  collapsed state; the destination is visibly unavailable and does nothing when chosen.
- Every transition MUST be reachable by keyboard (FR-007).
