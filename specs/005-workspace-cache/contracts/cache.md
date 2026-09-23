# Contract: `WorkspaceCache`

**Feature**: F003 workspace-cache | **Date**: 2026-09-23

The projection as a capability. §5.2 is the source of truth for the schema; data-model.md maps
entities onto it. This document states what the port promises, so that `CachedWorkspace`'s rules
can be written against a promise rather than against SQLite.

The port is a **capability, not a technology** (Principle VIII): it is `WorkspaceCache`, not
`SqliteWorkspaceStore`. Two implementations ship — the SQLite adapter and an in-memory fake.

---

## Operations

| Operation | Promise |
|---|---|
| `register(workspace)` | Idempotent. Registering an existing id attaches to its projection and updates `last_opened_at` (FR-011). Never creates a second projection |
| `forget(workspace_id)` | Removes tree and content together, by cascade (FR-012) |
| `list_children(workspace_id, parent)` | One indexed query, no join. Returns rows in `(is_directory DESC, name ASC)` — the same order the protocol promises |
| `put_listing(workspace_id, parent, entries)` | Replaces the children of `parent` atomically. `file_id` is preserved for entries that remain **under the same name**, so their content survives. An entry whose name is gone is removed and its content cascades away — see C9 |
| `lookup(workspace_id, path)` | The §5.4 open query: metadata and blob in one statement |
| `put_content(file_id, bytes, hash)` | Hashes first, compresses second (§5.6). Sets `is_cached` and `last_accessed_at` in the same transaction (invariants 3 and 9) |
| `touch(file_id, now)` | Records an access (FR-028) |
| `rename(file_id, new_path)` | Updates `relative_path`, `parent_path` and `name` only. Never touches `file_contents`, so content survives the move (FR-022). **This feature implements and tests it; its first caller is F006's write path** |
| `search_paths(workspace_id, fragment, limit)` | `files_fts`, never `LIKE` (§5.2, §5.4) |
| `evict(before)` | Deletes from `file_contents` only; clears `is_cached`. Never removes a `files` row (FR-027, §5.5) |
| `schema_version()` / `migrate_to(v)` | `PRAGMA user_version`; see the maintenance contract below |

---

## Guarantees

| # | Guarantee | Requirement |
|---|---|---|
| C1 | **Hash before compress.** `sha256_hash` is always over decompressed bytes, so it is directly comparable with the engine's | §5.6, FR-030 |
| C2 | **No blob above the eligibility cap.** `put_content` refuses 8 MiB+ content, returning a distinguishable "not eligible" rather than an error | Spec edge case; research.md |
| C3 | **Storing is best-effort.** Every write path can fail without the read failing. The port returns a result the caller is permitted to ignore, and `CachedWorkspace` ignores it | FR-034 |
| C4 | **Eviction never touches the tree.** Enforced by the statement, asserted against a real database | FR-027, SC-008 |
| C5 | **`is_cached` never lies.** Set and cleared in the same transaction as the blob it describes | invariant 3 |
| C6 | **Path search needs no network.** `search_paths` never consults a provider | FR-031, SC-011 |
| C7 | **Foreign keys are on for every connection.** Per-connection in SQLite, default off. A connection without it silently breaks C4's cascade | §5.2 |
| C8 | **The FTS index is maintained by trigger.** No caller updates it, and no caller may | data-model.md amendment |
| C9 | **`put_listing` cannot recognise a rename**, and does not pretend to. A re-listing sees one name gone and another present; linking them would require hashing every entry, which costs more than the refetch it saves. The vanished entry's content is dropped, the new entry is cached on next open, and **the file is listed throughout** | FR-022a, FR-022b |

C9 is the one that was nearly wrong. The opaque `file_id` (A-B5) exists so that a rename preserves
content, and it was tempting to write `put_listing` as though it delivered that. It cannot: identity
is preserved across a *known* move, and a re-listing does not know. The `rename` operation is where
`file_id` pays, and F006 is the first feature with a caller for it. Stating the limit here is what
stops a later reader assuming the cache handles renames it has never been told about.

C3 is the one with teeth. The natural signature returns `Result`, and the natural caller uses `?`.
That would turn a full disk into a failed file open, which FR-034 forbids. The contract test
fills the disk — by pointing the adapter at a store that refuses writes — and asserts the read
still returns bytes.

---

## Maintenance: the startup contract

Migration and eviction are one phase, run once, before anything else touches the projection.

```
open → read user_version
    v == current → evict(now - 14 days) → ready
    v <  current → migrate step by step, each in one transaction  FR-018, FR-018c
                     success → evict → ready
                     failure → discard file + -wal + -shm, recreate → ready  FR-018b
    v >  current → the file was written by a newer release → discard and recreate
```

| # | Guarantee | Requirement |
|---|---|---|
| M1 | No provider exists until this returns. The composition root constructs the cache, runs maintenance, and only then builds anything that reads | FR-018c, FR-026a |
| M2 | Each step is one transaction including its `user_version` bump. A crash mid-step rolls back to the previous version and the next launch retries the same step | FR-018b |
| M3 | A half-transformed projection is unobservable — not unlikely, **unreachable**. M2 is why | FR-018b, SC-013b |
| M4 | `-wal` and `-shm` are removed with the database. Deleting only the main file leaves a WAL SQLite will replay into the fresh one | research.md |
| M5 | `Migrating` and `Evicting` are published at least once per second throughout, and the developer is told when content was rebuilt | FR-018a, FR-018b, SC-013a |
| M6 | Eviction runs exactly once, here. Nothing evicts while a workspace is open | FR-026a, SC-008a |

M6 carries an accepted cost that FR-026b records: **a session that never restarts never evicts.**
Stated in the contract so nobody later reads the absence of a background reclaim as an oversight.

A newer-than-current `user_version` is handled by the same discard path. The alternative — refusing
to launch after a downgrade — strands a developer behind a disposable projection, which is the same
argument FR-018b makes for the failure case.

---

## What the fake must reproduce

The in-memory implementation is not a stub. To be useful for `CachedWorkspace`'s tests it must
reproduce C1 through C6, plus the ability to *fail on demand*: a refusing `put_content` (for C3), a
schema version it can be told to report (for M1 through M3), and a clock it does not own (so
retention is tested without waiting).

An in-memory fake that cannot fail tests only the happy path, and the happy path is not where
FR-034 lives.
