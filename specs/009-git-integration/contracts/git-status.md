# Contract: Git on the wire

Phase 1. What the engine and the client promise each other. §4.8 is the source of truth for the
table; this states the guarantees the table cannot express.

**Wire spelling is `snake_case`.** §4.8's tables are camelCase for readability and say so three
paragraphs below them (A-WIRECASE). Every field named here travels as `snake_case`.

---

## `git/getStatus` — request

**Params**: `workspace_id`, `cursor?`, `limit?`
**Result**: `{ current_branch, changes[], next_cursor? }`

### Guarantees

1. **`limit` defaults to and is capped at 1000.** A caller asking for more gets 1000; a caller
   asking for none gets 1000. The cap is the engine's, not a suggestion.
2. **`next_cursor` is present exactly when more entries remain.** Absent means this is the last
   page. A cursor is never present on an empty final page.
3. **Pages of one cursor chain describe one snapshot.** Paging never interleaves two repository
   states: the engine slices a computed result rather than re-running git per page.
4. **A cursor is opaque and single-workspace.** Presenting a cursor from one workspace to another
   is refused, not silently answered.
5. **An unknown or expired cursor is refused** with a code, rather than being treated as the
   beginning. Starting over silently would produce a picture assembled from two snapshots.
6. **`changes[]` entries are `{path, status}`**, `status` being exactly one of `MODIFIED`,
   `UNTRACKED`, `STAGED`, `DELETED`, `CONFLICT`. Never two, never absent.
7. **Paths are workspace-relative and contained.** A path escaping the root is not emitted; if
   git produces one, the engine drops the entry rather than forwarding it.
8. **A workspace that is not a repository, or a host without a usable git, answers successfully**
   with no branch and no changes — not an error. The distinction between the two is carried in
   the engine's own diagnostics, not in this reply, because the client's behaviour is identical.

### Refusals

| Condition | Code |
|---|---|
| Workspace not registered | `-32001` (§4.4) |
| Workspace root gone | `-32009` |
| Cursor unknown, expired, or from another workspace | `-32602` invalid params |

---

## `git/onStatusUpdate` — notification

**Params**: `workspace_id`, `current_branch`, `changes[]`, `next_cursor?`

### Guarantees

1. **It carries the first page**, because a notification cannot be answered. When it sets
   `next_cursor`, the client pulls the rest with `git/getStatus` (A-GITPAGE).
2. **It is emitted after a burst settles**, not per index write — one notification per coalesced
   burst (A-COALESCE, plan.md's *Fixed Quantities*).
3. **It is not emitted for a workspace with no registered client.**
4. **It never carries file content**, and never carries a path outside the workspace root.

### What the client must do with it, and what would be wrong

Applying the notification alone, when it sets a cursor, marks every path beyond the first page
as unchanged. The replacement commits only when the final page arrives; until then the previously
applied state remains visible. This is the single most likely way to implement this feature
incorrectly while every test that looks at one message passes.

---

## `git/getFileDiff` — request

**Params**: `workspace_id`, `relative_path`
**Result**: `{ added[], deleted[], modified[] }`

### Guarantees

1. **Coordinates only.** No file content appears in the result, in any field, ever (§12.3,
   FR-021). This is the guarantee to test by inspecting the payload, not by inspecting the code.
2. **Coordinates are one-based line numbers in the working copy**, matching what an editor
   numbers its lines.
3. **A deletion is a position, not a range** — the removed lines are not in the new file.
4. **An unchanged file returns three empty lists**, not an error.
5. **A file that is untracked returns every line as added**, because nothing has been recorded
   for it to differ from.

### Refusals

| Condition | Code |
|---|---|
| Path escapes the workspace root | `-32002` |
| Path does not exist | `-32003` |
| Workspace not registered | `-32001` |

---

## What is **not** in this contract

- **No write methods.** No commit, stage, unstage, branch, merge or revert. A client cannot
  change a repository through this protocol, and that is a property of the method list rather
  than of any check.
- **No history.** No log, blame, or file-at-revision.
- **No staged/unstaged pair.** One state per path; the derivation happens on the engine and is
  not reversible from the wire. A future feature wanting both would be a protocol change, made
  deliberately.

## Cross-boundary obligations

**Both sides validate paths** (Principle VI). The engine contains paths before emitting; the
client contains them again on arrival. A path from a subprocess is untrusted input exactly as a
path from the wire is, and the client's check protects against a bug in our own engine rather
than against a hostile one.

**Neither side lets git status touch cache validity** (§5.3). The engine does not invalidate on
status, and the client's application of an update must leave cached content and its hashes
untouched — a property worth asserting by counting cached files before and after.
