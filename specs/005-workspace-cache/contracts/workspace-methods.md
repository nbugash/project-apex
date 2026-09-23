# Contract: Workspace Read Methods

**Feature**: F003 workspace-cache | **Date**: 2026-09-23

The three §4.8 workspace methods this feature implements, on both ends. `project-apex-predator.md`
§4.8 is the source of truth for the method catalogue; this document states the guarantees the
catalogue's table has no room for, and marks the one place it must be amended.

Every method takes `workspaceId`. Every `relativePath` is **untrusted input** at the engine (§4.7).

---

## `workspace/readDirectory`

**AMENDS §4.8.** Two optional parameters and one optional result field are added. Per §4.8's own
versioning rule, adding optional parameters and result fields does **not** increment
`protocolVersion`.

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `relativePath` | string | yes |
| param | `cursor` | string | **no — new** |
| param | `limit` | integer | **no — new** |
| result | `items[]` | `{name, type, size, modified}` | yes |
| result | `nextCursor` | string | **no — new** |

### Guarantees

1. **Shallow.** Immediate children only. The engine never recurses (§10.1).
2. **Ordered, contractually.** `(type DESC, name ASC)` — directories before files, then by name,
   byte-wise on the UTF-8 encoding. This is the same order as §5.4's sidebar query, so the client
   renders what it receives without re-sorting.
   **The ordering is part of the contract because the cursor depends on it.** Changing it later is
   a breaking change and *would* increment `protocolVersion`.
3. **Paged.** At most `limit` entries, default and maximum 1000 (FR-024). `nextCursor` is present
   exactly when more entries follow, and is the `name` of the last entry returned.
4. **Resumable without server state.** A request carrying `cursor` returns the entries that sort
   strictly after it. The engine keeps no iterator, so a page may be requested at any time, in any
   order, after any restart.
5. **Concurrent modification is bounded.** An entry created between pages may be missed if it sorts
   before the cursor; an entry deleted between pages simply does not appear. No stable entry is
   ever duplicated or skipped. Offset paging guarantees neither, which is why the cursor is a name.
6. **No size limit breach.** 1000 entries at any plausible name length stays well inside §4.1's
   1 MiB cap.

### Errors

| Code | Condition |
|---|---|
| `-32002` | Path escapes the workspace root, lexically or after symlink resolution |
| `-32001` | Unknown or unregistered `workspaceId` (§4.4) |
| `-32003` | Path is inside the root and does not exist (§4.4) |
| `-32602` | `limit` is not a positive integer, or `cursor` is not a string |

`-32002` is returned identically whether the escaped target exists or not (FR-007).

---

## `workspace/register` — **NEW METHOD, AMENDS §4.8**

The catalogue has no way to tell the engine what a `workspaceId` means. §15.4 step 3 says
"Register the workspace with the engine" and §4.4 reserves `-32001` for "Workspace not found or
**not registered**" — so the system specification presumes this method in two places and defines
it in none. Every other method here is unusable without it.

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `path` | string (absolute, on the engine's host) | yes |
| result | `{name, canonicalPath}` | | |

Adding a method does **not** increment `protocolVersion` (§4.8). An older engine answers `-32601`,
which is exactly the signal a client needs to know it must redeploy.

### Guarantees

1. **Canonicalises once.** The root is resolved at registration and stored canonical, so every
   later request is a resolve and a prefix comparison rather than a second `canonicalize` of the
   root.
2. **Idempotent.** Registering an id that is already registered against the same path succeeds and
   changes nothing (A-WORKSPACE, FR-011). Against a *different* path it is an error, not a silent
   re-point: two meanings for one identity is the collapse FR-010 exists to prevent.
3. **Refuses a path that is not a directory, or not readable**, at registration rather than on the
   first read — so the failure names the workspace instead of a file inside it.
4. **In-memory only.** The registry dies with the engine. A client reconnecting to a restarted
   engine re-registers, and `session/onRestart`'s `unpreserved` list is how it learns it must
   (F002, §4.8).

`workspace/unregister` is deliberately **not** added. Deletion removes the remote directory and the
local cache (A-WORKSPACE) and is F00-series lifecycle work; dropping a registration without
deleting anything has no caller in this feature, and a method with no caller cannot be tested.

---

## `workspace/stat`

| | Name | Type |
|---|---|---|
| param | `workspaceId`, `relativePath` | string |
| result | `{type, size, modified, sha256}` | |

### Guarantees

1. **`sha256` is the whole-file hash of the current content**, over the raw bytes on disk. It is
   the only input to cache validity (FR-019, §5.3).
2. **`sha256` is absent for a directory.** There is nothing to hash and no caller that needs it.
3. **Cheap enough to be on the interaction path.** This is the call FR-021a makes before serving
   cached content, so it is the call the 2-second confirmation limit applies to
   (research.md, "The confirmation limit").
4. **A path that does not exist is not an error the caller should fear.** `stat` on a missing path
   inside the root returns `-32003`, which is information the caller is entitled to.

**Known cost, stated rather than discovered.** The engine hashes the file to answer this, so a
`stat` on a large file is proportional to its size. The eligibility cap bounds what the client
will ever `stat` for validity purposes, because content above it is never cached and so never
needs confirming.

---

## `workspace/readFile`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId`, `relativePath` | string | yes |
| param | `offset`, `length` | integer | no |
| result | `{content, encoding, sha256, totalSize}` | | |

### Guarantees

1. **Bytes, not text.** `encoding` is `"base64"`. There is no `"utf8"` path: FR-003 forbids
   assuming text, and a method that sometimes returns text and sometimes base64 makes every caller
   branch on it.
2. **`sha256` is of the whole file**, never of the returned range. A caller assembling several
   ranges compares this field across them; a change means the file moved underneath the read and
   the assembled result must be discarded (FR-021).
3. **`totalSize` is the whole file's size**, so a caller knows after the first range whether more
   follows.
4. **Bounded payload.** The engine refuses a request whose raw `length` exceeds **512 KiB** with
   `-32602`, rather than truncating silently. Omitting `length` on a file larger than that is the
   same refusal: the caller is expected to know the size from `stat` or from the directory listing.
   See research.md, "The bulk threshold".
5. **A range beyond the end of the file is not an error.** It returns zero bytes with the correct
   `totalSize`, which is what lets a caller scroll toward the end without a size race.

### Errors

| Code | Condition |
|---|---|
| `-32002` | Path escapes the root |
| `-32001` | Unknown or unregistered `workspaceId` |
| `-32602` | `length` exceeds 512 KiB, or the file exceeds it and no range was given |
| `-32007` | Frame cap — should be unreachable given the 512 KiB refusal; retained as defence |
| `-32003` | Path inside the root does not exist |

---

## Bulk reads: not a protocol method

Content above the threshold does **not** travel through this channel at all (A-BULK, §3.6, §4.6).
It is fetched by a separate `ssh` invocation attached to the existing control master.

Two constraints carry over from F002, both learned the hard way there:

- The invocation MUST pass `ControlMaster=no`. A bulk invocation that becomes the master
  backgrounds itself while holding the inherited stdout pipe, and reading its output then waits
  forever for an EOF that cannot arrive.
- Integrity is the caller's job. The bulk path returns bytes with no hash; the caller compares
  against the `sha256` from `stat` and discards a mismatch. Nothing about the transfer authenticates
  the content.

---

## What is NOT implemented here

Declared by §6.1 and §4.8, refused by this feature with a stated reason rather than silently
absent (FR-004):

`workspace/writeFile`, `workspace/createFile`, `workspace/createDirectory`, `workspace/rename`,
`workspace/delete` — F006, which owns `baseSha256` conflict handling.

`workspace/watch` and its notifications — F004.

Content search — F013. Offline **path** search is in scope and is served locally from `files_fts`,
never as a protocol method.

A refusal is `-32601` (method not found) from the engine and a typed `Unsupported` error from the
client-side provider. Both name the feature that will implement it, because a developer reading a
log deserves to know whether they found a bug or a schedule.
