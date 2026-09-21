# Contract: Rail Commands (interface layer ↔ core)

**Branch**: `002-shell-chrome-fidelity` | **Date**: 2026-09-21

Additions to the `ShellCommands` inbound port defined in
[`../../001-app-shell/contracts/shell-commands.md`](../../001-app-shell/contracts/shell-commands.md).
Everything in that contract still applies: input arriving here is untrusted regardless of
interface-layer validation, and the core rejects rather than coerces.

Entity shapes are in [data-model.md](../data-model.md) and are not restated.

---

## Commands (interface layer → core)

### `rail_select(destination_id: String) -> Result<ToolWindowState, ShellError>`

Selects a rail destination. Selecting the destination that is already active toggles the tool
window collapsed or expanded.

- **Precondition**: `destination_id` names a known destination.
- **Postcondition**: returns the resulting tool window state; the session is scheduled for
  persistence. Selecting an `unavailable` destination is a no-op that returns the unchanged
  state — not an error, because the destination is legitimately present and the interface
  already shows it as unavailable.
- **Errors**: `UnknownDestination`.

### `tool_window_resize(width: u32) -> Result<(), ShellError>`

- **Precondition**: `width ≥ MIN_TOOL_WINDOW_WIDTH`.
- **Postcondition**: width recorded; persistence scheduled, not awaited.
- **Errors**: `WidthBelowMinimum`.

### `rail_destinations() -> Vec<RailDestination>`

Returns the rail's destinations in order, each with its availability.

- **Precondition**: none.
- **Postcondition**: no state change.
- **Errors**: none.

---

## Error additions

```text
ShellError
├── UnknownDestination     no rail destination with that identifier
└── WidthBelowMinimum      requested tool window width below the minimum
```

Both follow the existing convention: a live command rejects an out-of-range value, while
loading a stale file repairs it. That asymmetry is deliberate and is recorded in the shell's
architecture; it is restated here only because a reader arriving at this contract first would
otherwise see it as an inconsistency.

---

## Non-goals for this contract

**Tool window content.** These commands carry which destination is active and how wide the
panel is. What each destination *displays* belongs to the feature that owns it.

**Rail composition.** The destination list is static and derived from the prototype; there is
no command to add, remove or reorder one. A registry was considered and rejected — see
[research.md](../research.md), "Where the rail's destinations come from".
