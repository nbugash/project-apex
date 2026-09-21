# Data Model: Application Shell

**Branch**: `001-app-shell` | **Date**: 2026-09-21 | **Plan**: [plan.md](./plan.md)

Entities this feature owns. Field-level definitions live here and are not restated in
`design.md`. Persistence format is fixed by
[contracts/session-state.schema.json](./contracts/session-state.schema.json).

---

## Constants

Referenced by the validation rules below and by
[contracts/session-state.schema.json](./contracts/session-state.schema.json). Fixed here so
that every artifact and every test asserts the same threshold.

| Constant | Value | Basis |
|----------|-------|-------|
| `MIN_REGION_EXTENT` | 120 px | Below this a navigation tree shows only truncated names and an output console fewer than three lines, so the region is present but useless. Regions are hidden rather than shrunk past it |
| `MIN_WINDOW_WIDTH` | 800 px | Navigation at its minimum, plus a document area wide enough for an 80-column line at the design system's monospace size |
| `MIN_WINDOW_HEIGHT` | 600 px | Tab strip, a usable document area, and the status bar, with the output region at its minimum |

These are enforced by clamping on load and by rejection on a live command — an asymmetry
recorded in [architecture.md](./architecture.md) Phase 1 Reconciliation.

---

## PersistedSession

Root aggregate. Everything the shell restores on launch. One instance per installation.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `schema_version` | integer | yes | Starts at 1. Incremented when the shape changes incompatibly |
| `workspace` | WorkspaceReference \| null | yes | `null` when no workspace has been opened |
| `window` | WindowGeometry | yes | |
| `layout` | Layout | yes | |
| `documents` | OpenDocumentReference[] | yes | May be empty |
| `focused_document_id` | DocumentId \| null | yes | `null` iff `documents` is empty |

**Validation**

- `focused_document_id`, when non-null, MUST match the `id` of an entry in `documents`.
  A dangling reference makes the whole file invalid (FR-008).
- An unreadable, malformed, or higher-`schema_version` file is discarded in full and replaced
  by defaults. Partial recovery is explicitly not attempted — a half-restored layout is harder
  to reason about than a default one (FR-008, SC-003).
- Discarded state is overwritten on the next save, not re-read on each launch (Edge Cases).

---

## WorkspaceReference

What the status area displays. The shell holds the reference; content belongs to F003.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `name` | string | yes | Display name, 1–255 characters |
| `location_type` | `"REMOTE"` \| `"LOCAL"` | yes | Mirrors the system specification's workspace model |

**Validation**

- `name` is displayed in a fixed-width status region and MUST be truncated for display
  without altering the stored value (FR-013).

---

## WindowGeometry

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `x` | integer | yes | Screen coordinate, may be negative on multi-display setups |
| `y` | integer | yes | |
| `width` | integer | yes | ≥ `MIN_WINDOW_WIDTH` |
| `height` | integer | yes | ≥ `MIN_WINDOW_HEIGHT` |
| `maximized` | boolean | yes | When true, `x`/`y`/`width`/`height` record the pre-maximize rectangle |

**Validation**

- Restored geometry is accepted only if the window's title bar falls within the working area
  of an attached display. Otherwise the default geometry on the primary display is used
  (FR-009, SC-009). The threshold is the title bar specifically, because a window whose title
  bar is off-screen cannot be dragged back by the user.
- `width` and `height` below their minimums are clamped rather than rejected; the file stays
  valid.

---

## Layout

Region arrangement. Exactly three regions exist in this feature; the set is closed.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `navigation` | RegionState | yes | Project navigation, left |
| `output` | RegionState | yes | Output and consoles, lower |
| `document_area` | RegionState | yes | Primary area; see constraint below |

**Validation**

- `document_area.visible` MUST be `true`. The primary area is not hideable — a shell with no
  document area is not a usable state to persist or restore.
- Any region with `visible: true` MUST have `extent ≥ MIN_REGION_EXTENT`. Values below it are
  clamped on load (FR-004).

---

## RegionState

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `visible` | boolean | yes | |
| `extent` | integer | yes | Pixels along the region's variable axis: width for navigation, height for output |

**Validation**

- `extent` is retained while `visible` is `false`, so hiding and re-showing a region restores
  its previous size rather than a default.

---

## OpenDocumentReference

A pointer to something shown in the document area. The shell owns the reference and its
ordering; content is a later feature's concern.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `id` | DocumentId | yes | Opaque, stable for the lifetime of the tab |
| `display_name` | string | yes | Tab label, 1–255 characters |
| `order` | integer | yes | Zero-based position in the tab strip |

**Validation**

- `id` MUST be unique within `documents`.
- `order` values MUST form a contiguous zero-based sequence. Gaps or duplicates invalidate the
  file, since the tab strip cannot be rendered unambiguously from them.

**Identity note**

`id` is opaque and is not derived from a path. A document whose path changes keeps its tab,
its position and its focus. This mirrors the identity correction made to the workspace cache
in A-B5, for the same reason: deriving identity from location makes a rename destroy state
that the user expects to survive it.

---

## ConnectionState

Runtime only. Never persisted — a connection state restored from disk would be a claim about
the present made from stale information.

| Value | Meaning |
|-------|---------|
| `Unknown` | No report received yet; the state at launch |
| `Connecting` | An attempt is in progress |
| `Connected` | The engine is reachable and responsive |
| `Disconnected` | Not reachable |

**Validation**

- Each value MUST be distinguishable by shape or text in addition to colour (FR-012, SC-007).
  Colour alone is not a sufficient encoding.

**Transitions**

```text
Unknown ──> Connecting ──> Connected
                │              │
                └──> Disconnected <──┘
                         │
                         └──> Connecting
```

In this feature every transition is driven by the stub adapter. F001 replaces the source; the
state set and the transitions do not change, which is what makes the swap a one-adapter
change.

---

## SessionLifecycle

Not persisted. The shell's own startup state, which the readiness gate depends on.

| Value | Meaning |
|-------|---------|
| `Loading` | Reading persisted state; window created but not shown |
| `Restored` | Persisted state applied successfully |
| `Defaulted` | Persisted state absent or invalid; defaults applied |
| `Ready` | Styles applied, first render committed, window shown |

**Validation**

- The window MUST NOT become visible before `Ready` (FR-020, SC-011).
- Both `Restored` and `Defaulted` proceed to `Ready`. Falling back to defaults is a normal
  path, not an error path, and MUST NOT surface an error to the user (FR-008).
