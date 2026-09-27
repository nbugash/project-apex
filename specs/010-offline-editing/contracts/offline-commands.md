# Contract: what the interface may ask the core about offline work

Phase 1. These are Tauri commands, not protocol methods — the boundary between the webview and
the client core. They are stated as a contract for the same reason the protocol is: **the
workspace is resolved in the core and never accepted from the view** (Principle VI), and that is
a promise the view cannot check.

---

## `offline_status` — request

**Params**: none
**Result**: `{ connected, pending[] }` where each entry is `{ relative_path, mergeable }`

### Guarantees

1. **`connected` comes from the connection state F001 publishes** and is never re-derived here
   (FR-001). Two authorities on whether the client is online is the defect this avoids.
2. **Reading this never contacts the engine.** It reports what the client holds. An outage
   therefore costs nothing and times out never, which is the same rule `git_status` follows.
3. **`pending` is every file with retained work in the current workspace**, and nothing else. A
   file the developer edited and that has since reached the host does not appear.

---

## `conflicts_list` — request

**Params**: none
**Result**: `conflicts[]` of `{ relative_path, base, local, remote }`

### Guarantees

1. **Three versions, always.** A conflict the developer cannot see all three sides of is one they
   cannot decide, and `base` is the one a two-way interface would omit.
2. **`remote` is read when the list is built**, not stored. A stored remote side can go stale
   while the developer is looking at it, and resolving against a stale remote is how a merge
   silently discards a change that arrived during the conversation.
3. **`base` is empty only for a file created offline**, which genuinely has nothing it was
   derived from. Eviction of the cached content does **not** empty it: the base travels with the
   pending edit rather than being referenced (FR-011b). This guarantee said "a file with no cached
   text has `base` empty" while the base still came from the cache, and was left behind when that
   changed — analyze run 4 found it.
4. **A file the client cannot merge is still listed**, with `base` present or empty on the terms
   above, because FR-025a requires it to prompt.

---

## `conflict_resolve` — request

**Params**: `relative_path`, `resolution`
**Result**: `{ outcome }`

### Guarantees

1. **The resolution is written to the host and the retained work is deleted in one transaction.**
   Neither happens without the other, because a deleted pending edit whose write failed is lost
   work and a kept one whose write succeeded is a phantom conflict.
2. **A write refused for a stale base is not an error but a new conflict.** The host moved again
   while the developer was deciding; they are asked again against the newer remote rather than
   told their resolution failed.
3. **Resolving one file never touches another.** Per-file, as FR-023 requires.

---

## Cross-boundary obligations

**The view never names a workspace.** Every command here resolves the current workspace in the
core, as `git_status` and `git_file_diff` do. A workspace identity arriving from the view is an
identity the view could choose.

**Paths are re-validated on the way out of the store.** A `pending_edits` row is data the client
wrote, but it is data that has been on disk between two runs of the application, and Principle VI
does not make an exception for a boundary that is merely slower.
