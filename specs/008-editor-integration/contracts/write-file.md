# Contract: `workspace/writeFile`

**Feature**: F006 `editor-integration` | **Date**: 2026-09-26

`project-apex-predator.md` §4.8 is the source of truth for the catalogue. This document states
the guarantees its table has no room for. Where the two disagree, §4.8 wins and this file is
wrong.

The method exists in §4.8 and returns `METHOD_NOT_FOUND` today. This feature implements it.

---

## Shape

Request, per §4.8:

| Field | Type | Notes |
|---|---|---|
| `workspaceId` | string | Which workspace owns the path. Validated as every other workspace method validates it. |
| `relativePath` | string | Workspace-relative. **Untrusted**, like every path off the wire (§4.7). |
| `content` | string | The whole file. §4.8 carries content, not a patch. |
| `baseSha256` | string | The hash the client believed current when it began editing. |

Result: `{ "sha256": "<hash of what was written>" }`.

**Wire spelling is snake_case** (§4.8): `workspace_id`, `relative_path`, `base_sha256`. The
tables are written camelCase for readability; see A-WIRECASE, which records why that is not a
divergence and how counting spellings led to the wrong conclusion once.

---

## Guarantees

1. **The base is compared before anything is written.** If the file's current hash differs from
   `baseSha256`, the engine replies `-32004` and the file is untouched. Not "is restored" —
   untouched: nothing was opened for writing.

2. **A write is whole or it is nothing.** Content is written to a temporary file in the
   destination's own directory and renamed into place. A failure at any point leaves the
   previous content byte-for-byte intact (FR-015). The temporary file shares a directory with
   the target so the rename cannot cross a filesystem boundary, which would make it a copy and
   lose the property.

3. **The path is canonicalised and contained, by the engine, independently.** The same
   `resolve_request` the read methods use. A path escaping the workspace root is `-32003`,
   including by symlink, because canonicalisation resolves links before the comparison
   (Principle VI).

4. **Content is bounded before the work.** Above the stated maximum the request is refused
   rather than buffered. §4.1's 1 MiB frame cap already bounds a single request; the use case
   bounds it too, so the protection does not depend on the codec being the only caller.

5. **The returned hash is of what was written**, computed after the write, not from the request.
   A client adopting it as its new base is adopting what is on disk.

6. **The file's mode is preserved.** A rename replaces the inode, so the new file is created
   with the previous file's permissions rather than the process default — otherwise saving would
   silently strip an executable bit.

7. **No write is reported before it completes.** The reply is sent after the rename returns.

---

## Errors

| Code | When | §4.4 |
|---|---|---|
| `-32004` | `baseSha256` does not match current content | Write conflict |
| `-32003` | Path escapes the workspace root | Path refused |
| `-32001` | Workspace not registered | |
| `-32009` | Workspace root has vanished | |
| `-32602` | Malformed params | Invalid params |

`-32004` is reserved for the base mismatch and means only that. A missing file, an unreadable
one and a permission failure are not conflicts, and collapsing them would make the client's
"someone else edited this" message a lie in three other situations.

---

## What the client may assume

- A `-32004` means **the host's content differs from the base it sent**. It does not say what
  changed or who changed it; finding out costs a `stat` or a read.
- A success means the bytes are on disk and the returned hash describes them.
- Anything else means the write did not happen. There is no partial success.

## What the client may not assume

- That a success means no one else will write a moment later. The base check is at the instant
  of writing, not a lock.
- That `-32004` will still be true if retried. The host may have moved again.
