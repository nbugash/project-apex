# Data Model: Offline Editing

**Branch**: `feature/F012-offline-editing` | **Date**: 2026-09-27 | **Plan**: [plan.md](./plan.md)

The client gains one table. The engine gains nothing: it already knows everything this feature
asks it, and the only new method is a read.

---

## `pending_edits` (new, schema version 4)

What the developer has written that the host has not seen.

| Column | Type | Notes |
|---|---|---|
| `workspace_id` | TEXT NOT NULL | Part of the key. FK to `workspaces`, `ON DELETE CASCADE` |
| `relative_path` | TEXT NOT NULL | Part of the key. `/src/main.rs`, rooted, as every other path in this schema |
| `content_blob` | BLOB NOT NULL | The saved offline content, Zstd level 3, as `file_contents` stores content |
| `base_blob` | BLOB | The content this was derived from, Zstd level 3. **NULL for a file created offline.** Stored, not referenced: a three-way merge needs the base *text*, and the only other copy lives in `file_contents`, which is evictable and is overwritten by any refetch |
| `base_sha256` | TEXT | Hash of `base_blob`. NULL on the same terms. Kept alongside the content so a fast-forward is a hash comparison and never a decompression |
| `mergeable` | INTEGER NOT NULL | 0 when the client does not hold the file as text or it exceeds the editor's limit; such a file always prompts (FR-025a) |
| `retained_at` | INTEGER NOT NULL | Unix seconds. For ordering the reconciliation report, not for deciding anything |

**Primary key**: `(workspace_id, relative_path)`.

**Why keyed by path and not by `file_id`.** A file created offline has no `files` row and no
`file_contents` row, so there is no identity to key on. F011 met this shape from the other side:
git status was keyed by the tree's identity for a file and could not describe an untracked file in
a folder the tree had never listed. One instance of that lesson is enough.

**Why the base content is stored rather than referenced.** This was the design's one real hole,
found by analyze run 2 while tracing the "cached content was evicted" edge case. The first
version stored `base_sha256` alone, which is enough to detect that the host has not moved but
**not** enough to merge: a three-way merge needs the base text. The only other copy is in
`file_contents`, which A-PENDING itself points out is evictable — and which is also overwritten
whenever the client refetches the path. So on the very path the merge exists for, the base would
have been gone. All three design artifacts agreed with each other and were wrong together, which
is why a consistency check did not find it.

**Why a separate table and not columns on `file_contents`.** A pending edit must survive eviction
of the cached content it came from, must exist where there is no cached content at all, and
answers a different question — "what has the developer written" rather than "what did the host
last give me". Recorded in [research.md](./research.md).

**Validation rules**

- `relative_path` is re-validated as a contained, rooted path on read as well as on write
  (Principle VI). A row whose path does not validate is dropped, not repaired.
- `base_sha256` is either a valid hash or NULL. An empty string is not a valid value and is
  treated as a malformed row.
- A row exists only while the work is unreconciled. It is deleted in the same transaction that
  commits the write to the host (FR-022).
- `base_blob` and `base_sha256` are written **once**, when the path first gains a pending edit. A
  later offline save replaces `content_blob` and leaves both untouched: re-deriving the base from
  the new local content would make the merge compare local against local, which produces a wrong
  answer rather than an error (FR-011b).
- Either both base columns are set or both are NULL. One without the other is a malformed row.

**What this table must never do.** It must not participate in cache validity. §5.3 says a cached
blob is valid when its hash equals the engine's current hash for the path, and nothing else; a
pending edit says nothing about whether `file_contents` is still current.

---

## `file_contents` (existing, unchanged)

Named here only to record that it **is** unchanged. A-OFFLINE's Consequences say a base revision
becomes part of the cache schema, which could be read as adding a column here. It is not: the base
belongs to the pending edit, because it describes what the developer's work was derived from
rather than what the cache holds.

---

## Reconciliation outcome (in memory, not stored)

What happened to each file on reconnection, reported to the developer (FR-024) and then discarded.

| Field | Notes |
|---|---|
| `relative_path` | The file |
| `outcome` | `FastForwarded`, `Merged`, `Conflicted`, `NotAttempted`, `Failed` |
| `detail` | For `Failed`, why; empty otherwise |

Not persisted. A conflict that must survive a further disconnection survives as its
`pending_edits` row, not as an outcome — the row is the durable fact, and the outcome is a
statement about one reconciliation attempt.

**State transitions per file**

```text
retained ──(host hash == base)────────────▶ FastForwarded ──▶ row deleted
   │
   ├──────(host moved, no overlap)────────▶ Merged ────────▶ row deleted
   │
   ├──────(host moved, overlap)───────────▶ Conflicted ────▶ row kept, awaiting the developer
   │
   ├──────(not mergeable)─────────────────▶ Conflicted ────▶ row kept  (FR-025a: always, even
   │                                                                     when the host has not moved)
   │
   └──────(connection lost mid-way)───────▶ NotAttempted ──▶ row kept, retried next reconnection
```

A row is deleted **only** on a write the host confirmed. Every other path keeps it, which is what
makes FR-022 true by construction rather than by care.

---

## Conflict (in memory, held while unresolved)

The three versions shown to the developer: the base content, the local content, and the host's
current content. Reconstructed from the `pending_edits` row plus a fresh read of the host, rather
than stored — storing the host's side would create a fourth version that can go stale while the
developer looks at it.

---

## Prefetch candidate (in memory)

A path prefetch has decided is worth caching: the path and where it came from (`recent-commit` or
`manifest`). Transient, never stored. Prefetch keeps no record between runs because the cache is
already the record of what it achieved.

---

## Migration: version 3 to version 4

Additive only — one `CREATE TABLE`. Nothing existing is dropped, re-keyed or rewritten, unlike
F011's V3, which had to re-key `git_status`.

The migration test asserts what the V3 test learned to assert: that **everything else survives**.
Nothing has ever written a `pending_edits` row at migration time, so a migration that dropped the
database and recreated it would satisfy every check about the new table perfectly. What must
survive is the workspaces, the files, the cached content and the git state F011 added.
