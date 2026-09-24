# Data Model: Workspace Cache

**Branch**: `feature/F003-workspace-cache` | **Date**: 2026-09-23 | **Plan**: [plan.md](./plan.md)

Two models, one boundary. The **persisted** model is §5.2's canonical schema, which this feature
creates for the first time. The **domain** model is what the application layer manipulates, and it
deliberately does not mirror the tables: a row is a storage shape, and the rules in FR-019 through
FR-034 are about entities.

Everything below is derived from §5.2, §5.3, §5.5, §5.6, §6.1, A-B5 and A-WORKSPACE. Where a
constraint has a source, it is named. Where a constraint is new, it says so.

---

## Domain entities

### `WorkspaceId`

A newtype over a UUID, minted client-side (A-WORKSPACE).

| Rule | Source |
|---|---|
| Stable across sessions and process restarts | FR-009 |
| Independent of the display name; names may collide freely | FR-010, A-WORKSPACE |
| Minted by the client before the engine has seen the workspace | A-WORKSPACE |
| Mandatory on every workspace protocol method | §4.8 |

### `Workspace`

| Field | Type | Rule |
|---|---|---|
| `id` | `WorkspaceId` | Primary identity |
| `name` | `String` | Display only. Repository directory name. May collide (A-WORKSPACE) |
| `location` | `Location` | `Remote { host: String, base: AbsPath }` or `Local { base: AbsPath }` |
| `last_opened_at` | `Timestamp` | Updated on attach |

State transitions: `Unregistered → Registered → Attached → Detached`, and `Registered → Deleted`.
Attaching an already-registered workspace re-uses its projection rather than creating a second one
(FR-011). Deletion removes cached content *and* tree (FR-012), which the schema's
`ON DELETE CASCADE` performs.

### `RelPath`

A workspace-relative path, validated on construction (§6.2).

| Rule | Source |
|---|---|
| No component may be `..`; the path may not be absolute | §4.7, FR-005 |
| Validation on the client protects against bugs, never against a stale or hostile client | FR-008 |
| The engine re-derives its own; it never receives this type | FR-008, Principle VI |

Normalised form: leading `/`, `/` separators, no trailing `/` except for the root, which is `/`.
The normalised string is what is stored in `files.relative_path` and what a cursor is compared
against.

### `FsEntry`, `FsMeta`, `ByteRange`, `FileChunk`

The four shapes §6.1's trait signature already fixes. They are listed here because the persistence
mapping needs names, not because this feature gets to choose them.

| Type | Fields | Note |
|---|---|---|
| `FsEntry` | `name`, `kind: File \| Directory`, `size: u64`, `modified: Timestamp` | One directory child |
| `FsMeta` | `kind`, `size`, `modified`, `sha256: Option<Sha256>` | `sha256` is `None` for a directory |
| `ByteRange` | `offset: u64`, `length: u64` | `None` in `read_file` means the whole file |
| `FileChunk` | `bytes: Vec<u8>`, `range: ByteRange`, `total_size: u64`, `sha256: Sha256` | **Bytes, never text** (FR-003, §6.1) |

`FileChunk.sha256` is the hash of the **whole file**, not of the chunk. A chunk carries it so that
a caller assembling ranges can tell that every range came from the same version of the file, which
is what FR-021's internal-consistency rule requires.

### `Sha256`

A newtype over 32 bytes, rendered lowercase hex on the wire. Computed over **decompressed**
content (§5.6) so it compares directly with the engine's.

### `CacheEntry`

The application-layer view of one cached file. Not a table.

| Field | Type | Rule |
|---|---|---|
| `file_id` | `FileId` | Opaque UUID, **not** derived from the path (A-B5) |
| `path` | `RelPath` | Mutable — a **known** rename updates it in place, leaving content untouched (FR-022). A rename merely observed in a re-listing is not known; see below |
| `hash` | `Sha256` | Of the decompressed content |
| `bytes` | `Vec<u8>` | Decompressed |
| `last_accessed_at` | `Timestamp` | Updated on every hit (FR-028) |

### `Validity`

The whole of FR-019 as a type, so that "nothing else invalidates it" is structural rather than a
comment.

```
Validity = Valid | Stale | Absent
```

Derived **only** from comparing `CacheEntry.hash` with the engine's `FsMeta.sha256`. Nothing else
constructs a `Validity`. In particular there is no git input: FR-020 and §5.3 make git status a
presentation signal, and the type having no constructor that takes one is what enforces it.

### `Presentation`

What the interface is told about content it is about to show. New in this feature; the requirements
that create it are FR-021a/b/c and FR-032.

| Variant | When | Source |
|---|---|---|
| `Verifying` | Connected, confirmation outstanding | FR-021b |
| `Current` | Confirmed against the engine this open | FR-021a |
| `Unverified` | Confirmation exceeded its limit | FR-021c |
| `PossiblyStale` | Served while disconnected | FR-032 |
| `Unavailable` | Disconnected and not cached | FR-033 |
| `Gone` | The workspace's root no longer exists on the engine | FR-038 |

These are the states the status surface renders. They are not cache states: `Unverified` and
`PossiblyStale` describe the same bytes with the same hash and differ only in why nobody could
confirm them.

`Gone` is the only one that is not about *content*. The other five qualify bytes the developer is
about to see; `Gone` says the thing being projected does not exist, so there is nothing to qualify.
That is why it is a variant rather than a flag on `Unavailable`: offline-and-uncached is a fact that
will stop being true when the connection returns, and a deleted workspace is not.

### `MaintenancePhase`

Published during startup maintenance. New in this feature (FR-018a, FR-026a).

```
Idle → Checking → Migrating { from: u32, to: u32 } → Evicting → Ready
                ↘ Evicting (version current)        ↗
                  Migrating → Rebuilding ───────────┘
```

Six states, and the names are canonical — [design.md](./design.md)'s state diagram and the task that
renders them use these and no others.

| State | Meaning | Rendered? |
|---|---|---|
| `Idle` | Before maintenance begins | no |
| `Checking` | Reading `PRAGMA user_version` | no — too brief to be worth a frame |
| `Migrating { from, to }` | A schema step is running | **yes** (FR-018a) |
| `Rebuilding` | A migration failed; the projection is being discarded and recreated | **yes** (FR-018b) |
| `Evicting` | Retention is reclaiming content | **yes** |
| `Ready` | Maintenance complete. **The precondition for constructing any provider** (FR-018c, M1) | no |

`Migrating` and `Evicting` refresh at least once per second while they run (FR-018a, SC-013a).
`Rebuilding` is entered only from a failed migration, or from a `user_version` newer than this
build (FR-018b).

### `RetentionWindow`

Fourteen days (§5.5, FR-026). A value rather than a constant so eviction is testable against a fake
`Clock` without waiting a fortnight.

---

## Persisted schema — version 1

Exactly §5.2, with the amendment research.md records. Reproduced here only where this feature adds
to it; §5.2 remains the source of truth for the rest and this document does not restate it.

`PRAGMA user_version = 1` identifies the shape. The pragmas of §5.2 — `journal_mode = WAL`,
`synchronous = NORMAL`, `foreign_keys = ON` — are set on every connection, not only at creation:
`foreign_keys` is per-connection in SQLite and defaults off, so a connection that forgets it gets
cascade-free deletes and a projection that leaks rows.

### Amendment: FTS5 synchronisation triggers

`files_fts` is declared `content='files'`, which makes it external-content and means SQLite does
**not** maintain it. Without these, the table is created empty, stays empty, and every offline path
search returns no rows with no error. See research.md, "The FTS5 synchronisation gap in §5.2".

```sql
CREATE TRIGGER files_fts_insert AFTER INSERT ON files BEGIN
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;

CREATE TRIGGER files_fts_delete AFTER DELETE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
END;

CREATE TRIGGER files_fts_update AFTER UPDATE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;
```

The `'delete'` command row is FTS5's external-content deletion protocol: the old values must be
supplied because the index cannot read them back from a row that is already gone.

**This must land in §5.2 of `project-apex-predator.md` before the code that depends on it.**

---

## Domain-to-table mapping

| Domain entity | Tables | Notes |
|---|---|---|
| `Workspace` | `workspaces` | One row. `ON DELETE CASCADE` gives FR-012 for free |
| Tree node | `files` | `UNIQUE(workspace_id, relative_path)` is what FR-011's attach relies on |
| `CacheEntry` | `files` ⋈ `file_contents` | The join in §5.4's open query |
| `Validity` | derived | Never stored. It is a comparison, and storing it would create a second truth |
| `Presentation` | not persisted | Per-open, per-connection state |
| Path search index | `files_fts` | Maintained by trigger only |
| Git status | `git_status` | Created at v1, written by F011. See research.md |

### Why `file_id` is opaque

A-B5's first correction. Path-derived identity meant a rename produced a new identity and orphaned
the blob. With an opaque UUID, a rename is an `UPDATE` of `relative_path`, `parent_path` and
`name`, and `file_contents` is untouched — which is FR-022 satisfied by the schema rather than by
code remembering to copy a blob.

**What that does not buy.** Opaque identity preserves content across a rename the projection is
*told about*. It does nothing for a rename discovered by re-listing a folder, because a re-listing
carries no identity at all — only a set of names, one gone and one new. Linking them would mean
hashing every entry in the folder, which costs more than refetching the one file. So the vanished
entry is removed, its content cascades away, and the file is re-cached on next open (FR-022a). The
entry stays listed throughout (FR-022b).

This matters because A-B5's correction reads like "renames are handled", and it is easy to write a
`put_listing` that claims so. `WorkspaceCache::rename` is where opaque identity actually pays, and
F006's write path is the first caller that can invoke it.

### Why `last_accessed_at` exists

A-B5's second correction. §5.5 evicts on time since last *read*; the original schema stored only
write time, so the policy was not expressible. FR-028 requires every hit to update it, which makes
retention measure use rather than age.

---

## Invariants

These are the things a test should be able to break and find something wrong. Each names the
requirement it enforces.

| # | Invariant | Enforced by | Requirement |
|---|---|---|---|
| 1 | `file_contents` never holds a row whose `files` row is gone | FK + cascade | FR-012 |
| 2 | Eviction deletes from `file_contents` only, never from `files` | Statement shape | FR-027, §5.5 |
| 3 | `is_cached = 1` ⟺ a `file_contents` row exists | Written in one transaction; asserted by an integration test | FR-027 |
| 4 | `sha256_hash` is over decompressed bytes | Compression happens after hashing, in that order | §5.6, FR-030 |
| 5 | No two `files` rows share `(workspace_id, relative_path)` | `UNIQUE` | FR-011 |
| 6 | `files_fts` contains exactly one entry per `files` row | Triggers | FR-031 |
| 7 | A projection is never read at a `user_version` the code was not built for | Maintenance runs before any provider is constructed | FR-018c |
| 8 | Nothing above the eligibility cap has a `file_contents` row | Write path checks size before compressing | Spec edge case; research.md |
| 9 | `last_accessed_at` is not null for any row with `is_cached = 1` | Set in the same statement that sets `is_cached` | FR-028, FR-026 |
| 10 | A file never disappears from the tree because its content went away — whether by eviction or by an unrecognised rename | Content removal clears `is_cached` and deletes from `file_contents` only | FR-027, FR-022b |

Invariant 3 deserves its own note: it is the one that makes `is_cached` meaningful as a *listing*
column. §5.4's sidebar query reads `is_cached` without joining `file_contents`, so a divergence
would show a file as cached in the tree and produce nothing when opened. That is why it is asserted
against a real database file rather than reasoned about.

---

## Engine-side state

The engine persists nothing. It holds one registry, in memory:

| Entity | Shape | Lifetime |
|---|---|---|
| `WorkspaceRoots` | `WorkspaceId → canonical AbsPath` | The engine process |

Canonicalised once at registration, so the per-request check is a resolve and a prefix comparison
rather than a second `canonicalize` of the root.

A root **deleted on the engine while registered** needs its own answer (FR-038). The registry holds
a canonical path that no longer resolves, so the next request against it fails — and failing as an
ordinary not-found would let the client keep browsing a projection of something that is gone, which
is the one case where the cache stops being a stale fact and becomes fiction. The engine therefore
distinguishes the two: a missing **root** is reported as the workspace being gone, not as a missing
path inside it.

That the registry is in memory carries F002's consequence unchanged: it does not survive a crash,
and a client that reconnects to a new engine re-registers. `session/onRestart`'s `unpreserved` list
is how the client learns, and workspace registrations belong in it.
