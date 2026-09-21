# Contract: Shell Commands (interface layer ↔ core)

**Branch**: `001-app-shell` | **Date**: 2026-09-21

The `ShellCommands` inbound port. This is the boundary the interface layer crosses to reach
the core, and the entry point every later feature extends.

**Trust**: input arriving here is untrusted regardless of interface-layer validation
(Constitution Principle VI). The core validates every argument and rejects rather than
coerces. Entity shapes are defined in [data-model.md](../data-model.md) and are not restated
here.

---

## Commands (interface layer → core)

### `shell_ready() -> Result<(), ShellError>`

Signals that the stylesheet has applied and the first render has committed. The core makes
the window visible in response.

- **Precondition**: lifecycle is `Loading`, `Restored` or `Defaulted`.
- **Postcondition**: lifecycle is `Ready`; the window is visible.
- **Idempotent**: a second call is a no-op, not an error. Interface-layer reloads during
  development would otherwise fail spuriously.
- **Errors**: none under normal operation.

### `session_get() -> Result<SessionSnapshot, ShellError>`

Returns the restored session, or defaults when persisted state was absent or invalid. The
caller cannot distinguish the two, by design — falling back is a normal path (FR-008).

- **Precondition**: none.
- **Postcondition**: no state change.
- **Errors**: none. Failure to read persisted state yields defaults, not an error.

### `layout_set_region(region: RegionId, visible: bool, extent: u32) -> Result<(), ShellError>`

Updates one region and persists the layout.

- **Precondition**: `region` is a known identifier. `extent ≥ MIN_REGION_EXTENT` when
  `visible` is true. `region` is not `DocumentArea` when `visible` is false.
- **Postcondition**: the region reflects the new state; the session is scheduled for
  persistence.
- **Errors**: `InvalidRegion`, `ExtentBelowMinimum`, `DocumentAreaNotHideable`.

### `documents_open(display_name: String) -> Result<DocumentId, ShellError>`

Opens a document reference and appends it to the tab strip.

- **Precondition**: `display_name` is 1–255 characters.
- **Postcondition**: a new reference exists at the highest `order`; it becomes focused.
- **Errors**: `InvalidDisplayName`.

### `documents_close(id: DocumentId) -> Result<(), ShellError>`

- **Precondition**: `id` refers to an open document.
- **Postcondition**: the reference is removed and remaining `order` values are re-packed to
  stay contiguous. If the closed document held focus, focus moves to its neighbour, or becomes
  `null` when none remain.
- **Errors**: `UnknownDocument`.

### `documents_reorder(id: DocumentId, to_order: u32) -> Result<(), ShellError>`

- **Precondition**: `id` refers to an open document; `to_order` is within
  `0..documents.len()`.
- **Postcondition**: ordering is contiguous with `id` at `to_order`.
- **Errors**: `UnknownDocument`, `OrderOutOfRange`.

### `documents_focus(id: DocumentId) -> Result<(), ShellError>`

- **Precondition**: `id` refers to an open document.
- **Postcondition**: `focused_document_id` is `id`.
- **Errors**: `UnknownDocument`.

---

## Events (core → interface layer)

Emitted, not requested. The interface layer subscribes at startup.

### `connection:changed`

Payload: `ConnectionState`. Emitted on every transition. In this feature the source is the
stub adapter; F001 replaces it without changing the event.

### `workspace:changed`

Payload: `WorkspaceReference | null`. Emitted when the active workspace changes, and once at
startup with the restored value.

---

## Error model

```text
ShellError
├── InvalidRegion            unknown region identifier
├── ExtentBelowMinimum       requested extent below the minimum for a visible region
├── DocumentAreaNotHideable  attempt to hide the primary area
├── InvalidDisplayName       empty, or longer than 255 characters
├── UnknownDocument          no open document with that identifier
├── OrderOutOfRange          target position outside the current tab count
└── PersistenceFailed        state could not be written
```

Every variant carries a human-readable message. `PersistenceFailed` is reported but MUST NOT
block the interaction that triggered it: a failed write loses persistence, and stopping the
user's resize because a file could not be written would turn a minor failure into a visible
one.

---

## Non-goals for this contract

Window geometry is not a command. The core observes native window move and resize events
directly, so the interface layer never reports positions it does not own.

Document *content* is absent entirely. This contract carries references and ordering only;
content arrives with the editor feature.
