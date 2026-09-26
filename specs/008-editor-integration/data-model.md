# Data Model: Editor Integration

**Feature**: F006 `editor-integration` | **Date**: 2026-09-26

What this feature holds, what makes each value valid, and how each changes. Rationale lives in
[research.md](./research.md); this file does not repeat it.

---

## Buffer

One open file's editable state. **At most one per file** (FR-023) — the rule that keeps bases
from diverging, because there is only ever one base to keep.

| Field | Type | Rules |
|---|---|---|
| `path` | workspace-relative path | Identity. Two opens of one path yield this same buffer. |
| `text` | string | What the developer sees and edits. Rendered locally; never awaits the engine (FR-002). |
| `base` | `Sha256` | The hash the content had when read, or the hash a successful write returned. Never set from any other source — see research.md, *Where the base hash lives*. |
| `dirty` | bool | Whether `text` has diverged from what was last written or read. |
| `loaded` | `LoadedRegions` | Which byte ranges are present. Whole for ordinary files. |
| `editable` | derived | False while `loaded` is incomplete (research.md, *Ranges*). |
| `ending` | `WriteOutcome?` | The result of the last save attempt, or none. |

### States

```
opened ──read──▶ clean ──edit──▶ dirty ──save ok──▶ clean
                   ▲                │
                   │                ├─ save refused (-32004) ──▶ dirty, conflicted
                   │                └─ save failed (transport) ─▶ dirty, unsaved
                   └──────────── discard and reload ────────────┘
```

**`conflicted` is not a separate state of the text.** The buffer is still dirty and still the
developer's; the conflict is a fact about the last attempt. The only transition out of it that
this feature offers is *discard and reload* (FR-012a), which replaces `text` with the host's
bytes and sets a new `base`. Merging is F012's.

### Validation

- `base` MUST be present before a save is attempted. A buffer with no base was never read.
- A save MUST NOT be issued while one is in flight for the same buffer (FR-014).
- `editable` false MUST reject edits rather than accept and discard them.

---

## LoadedRegions

Which byte ranges of a large file the buffer holds.

| Field | Type | Rules |
|---|---|---|
| `total` | `u64` | The file's size as last reported by `stat`. |
| `ranges` | list of `[start, end)` | Non-overlapping, ascending, merged on insert. |

- Complete when the merged ranges cover `[0, total)`.
- A range response shorter than requested is not an error: the file may have shrunk between
  requests. `total` is re-read rather than assumed.
- For a file at or below the chunk threshold this is always complete after one read
  (FR-018), so ordinary files never enter the partial path at all.

---

## AutosavePreference

| Field | Type | Rules |
|---|---|---|
| `enabled` | bool | Absent means **false** (FR-007b). |

Stored in the session store A-STATE defines, beside window geometry, layout, open documents and
task identities — not in the workspace cache, for the reason A-STATE gives: the cache is a
disposable projection and a preference is durable.

Adding it raises the store's schema version. The migration is the one A-STATE2 established:
`#[serde(default)]`, so a file written by the previous version parses and gets `false`, which is
also the correct default.

---

## WriteOutcome

What a save attempt produced. Distinguishable by construction, because FR-012 requires the
developer to be able to tell these apart.

| Variant | Means | Carries |
|---|---|---|
| `Written` | The engine accepted it | the new `Sha256`, which becomes the buffer's `base` |
| `Conflict` | `-32004`; the host's content changed since `base` | nothing — the host's current hash is fetched only if the developer asks to reload |
| `Refused` | The engine declined for another reason: path, permission, size | the code and its message |
| `Unreachable` | The request never completed | nothing |

`Conflict` and `Unreachable` are different things and are never collapsed: one means a colleague
edited the file, the other means the link dropped. A developer's next action differs completely.

---

## Quantities the plan fixes

Stated in [plan.md](./plan.md), *Fixed Quantities*, and referenced rather than repeated:
the chunk threshold, the scrolled range size, the autosave debounce, the maximum file opened as
text, and the count of extracted editor colours.

---

## What this feature does not model

- **No stored local revision.** F012 persists edits against the base held when the connection
  dropped; nothing here survives a restart but the tab list and the autosave preference.
- **No merge state.** Three-way merge is F012 and F019.
- **No cursor or scroll position.** Not named by A-STATE and not required; spec.md's
  *Assumptions* records the omission deliberately.
