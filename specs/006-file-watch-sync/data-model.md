# Data Model: File Watch Sync

**Branch**: `feature/F004-file-watch-sync` | **Date**: 2026-09-24 | **Plan**: [plan.md](./plan.md)

Three locations, one vocabulary. **Engine memory** holds what is being watched and what is
excluded, and dies with the process. **The wire** carries the two new requests and the two
notifications §4.8 already names. **The client projection** gains two columns and one rewrite it
did not previously need.

Rationale lives in [research.md](./research.md) and is not repeated here. Where a decision has a
source it is linked by heading; this document owns shape, not argument. Every entity names the
requirements that create it.

---

## Engine-side entities

### `Watch`

One directory the engine is observing. Never a file: see research.md, *Watch scope: what is
actually watched*.

| Field | Type | Rule |
|---|---|---|
| `workspace` | `WorkspaceId` | Mandatory on every workspace-scoped thing (§4.8) |
| `path` | `RelPath` | The watched **directory**, workspace-relative and normalised. Identity is `(workspace, path)`; there is no separate handle |

| Rule | Source |
|---|---|
| Must resolve, through the existing two-stage `ResolvedPath`, to a descendant of the canonical root | FR-002, §4.7 |
| Must be a directory; a file path is refused rather than silently watched by its parent | FR-003, research.md *Watch scope* |
| Consumes a host resource that is finite and shared per user | FR-005, spec Assumptions |
| Establishment happens inside the §1.4 budget, because expanding a folder is a developer-initiated interaction | plan.md, Performance Goals |

**Lifetime.** Created by `workspace/watch`. Released by `workspace/unwatch`, by the workspace
closing, by the connection dropping, and by the engine exiting (FR-004). The root's watch is the
exception: it is established at `workspace/register` and is not released while the workspace is
registered, because the root disappearing is the `-32009` condition and a workspace that has lost
its root must stop presenting a cached projection as a live view (research.md, *Watch scope*;
§4.4).

A watch that was **asked for and not established** is not a `Watch`. It is a refusal —
`{path, reason}` — returned in the call's result so that FR-005 is satisfied without failing the
call, which FR-005a requires.

The engine records no reason for holding a watch. Which folders are expanded and which files have
open tabs is knowledge only the client has (FR-003b), so the client computes the union and the
engine holds a flat set. That is why FR-004's compound release condition — a folder collapsing
*with no open tab inside it still requiring coverage* — is evaluated client-side and reaches the
engine as an ordinary `unwatch`.

### `WatchSet`

The per-workspace set the engine reconciles. **Idempotency is its defining property**, and every
other choice follows from it.

| Field | Type | Rule |
|---|---|---|
| `workspace` | `WorkspaceId` | One set per registered workspace |
| `watched` | `Set<RelPath>` | Directories. Normalised before insertion, so `/src` and `/src/` are one member, not two |

| Operation | Effect | Idempotence |
|---|---|---|
| `add(paths)` | Establishes a watch for each member not already present | Adding a member changes nothing and succeeds |
| `remove(paths)` | Releases the watch for each member present | Removing a non-member changes nothing and succeeds |

Idempotence is not politeness, it is what makes FR-026b honest. Reconnection re-establishes
watches for every folder still expanded and every file still open, and with set semantics that is
one `workspace/watch` carrying the current set — the engine reconciles against what it holds,
which after a restart is nothing. A per-path API would force the client to replay a remembered
history, and a client that mis-remembers resumes believing it is being told about changes when it
is not (research.md, *Protocol additions*).

**Invariants.** The root is a member from registration and cannot be removed while the workspace
is registered. Every member is an established watch: a refused path is reported and not added, so
the set never claims coverage it does not have — which is FR-025 expressed as a data structure
rather than as a message.

**Lifetime.** Engine memory, alongside the registry F003 built. It dies with the process, exactly
as the workspace registry does, and `session/onRestart`'s `unpreserved` list is how a client
learns it must re-send its set (FR-026b, plan.md Technical Context).

The client holds a mirror of what it has asked for, plus the refusals it was told about. Where
that mirror is surfaced is not settled: `ConnectionState` is the wrong home, because an exhausted
watch capacity happens while perfectly connected, and FR-005/FR-025 require the developer be told
in that case too. **Undetermined here; design.md's concern.**

### `RawEvent`

What the `FileWatcher` port yields, before coalescing. Shaped by what an inotify-class watcher can
observe, and named so that nothing outside `inotify_watcher.rs` learns which one it is (plan.md,
Structure Decision).

| Field | Type | Rule |
|---|---|---|
| `watch` | `RelPath` | The watched directory the event was reported against |
| `name` | `Option<String>` | The immediate child's name. `None` for an event about the watched directory itself |
| `kind` | `RawKind` | Below |
| `cookie` | `Option<u32>` | The rename pairing key. Present exactly on `MovedFrom` and `MovedTo` |
| `is_directory` | `bool` | Whether the subject is a directory. Needed because FR-022's subtree rewrite applies only to a directory rename |

```
RawKind = Created | Modified | Deleted | MovedFrom | MovedTo | SelfGone | Overflow
```

`MovedFrom` and `MovedTo` are deliberately **not** `Renamed`: at this layer the two halves have
not yet been paired, and pairing is the coalescer's job (research.md, *Rename detection*).
`SelfGone` is the watched directory itself disappearing, which for the root is the `-32009`
condition.

`Overflow` is the watcher's queue having dropped events. **What it maps to is undetermined.** It
resembles the wholesale case — nobody can say which paths were lost — but neither §10.4 nor
research.md addresses it, and FR-005's "report when it cannot watch" is the other candidate. This
document states the variant and declines to invent the mapping.

`RawEvent` carries **no timestamp**. The coalescer is fed events and told the time, because the
domain may not read a clock (plan.md, Constitution Check VIII); a `RawEvent` that stamped itself
would put `Instant::now()` back inside the layer the `Clock` port exists to keep it out of.

**Validation and exclusion.** `watch` joined with `name` must produce a path inside the canonical
root; the engine re-derives rather than trusts (§4.7, FR-002). Exclusion is enforced **twice**:
no watch is established on an excluded directory at all, and a raw event whose resolved path is
excluded is dropped before coalescing. The second check is not redundant — a file matching a
`.gitignore` rule can live inside a directory that is perfectly watchable, and FR-008 admits no
event for it.

**Lifetime.** Between the watcher thread's read and the coalescer's flush. A `RawEvent` never
crosses the wire and never reaches the client.

### `FileEvent`

What crosses the wire after coalescing — the payload of `workspace/onFileEvent`.

| Field | Type | Rule |
|---|---|---|
| `workspace` | `WorkspaceId` | FR-010: an event identifies its workspace |
| `kind` | `Created \| Modified \| Deleted \| Renamed` | FR-010: what happened |
| `path` | `RelPath` | FR-010: where. For `Renamed`, the **old** path |
| `to_path` | `Option<RelPath>` | Present **exactly when** `kind` is `Renamed` (FR-011) |

**It carries no content** (FR-013). It also carries no size, no modification time and no hash:
those are what a `workspace/stat` answers, and an event that carried them would be a second source
of truth competing with §5.3's hash comparison. An event says something changed, never what it now
is.

It carries no directory flag either, and does not need one: the client reads `files.is_directory`
for the old path out of its own projection, and if it holds no row there is nothing to rewrite
(FR-020). This is what lets the payload stay exactly the four fields §4.8 already defines.

| Invariant | Source |
|---|---|
| `to_path` present ⟺ `kind = Renamed` | FR-011 |
| At most one `FileEvent` per path per coalescing window | FR-012 |
| No `FileEvent` for an excluded path, ever | FR-008, SC-002 |
| An unpaired `MovedFrom` at flush becomes `Deleted`; an unpaired `MovedTo` becomes `Created` | research.md, *Rename detection* |
| Every arriving path is re-validated by the client against the root, independently of the engine | FR-014, SC-010, Principle VI |

That last row is the one with a client-side cost. The client's containment check is not a
duplicate of the engine's; it is the receiving end of a boundary, and SC-010 asserts the refusal
happens **including when the engine sent it**.

### `ExclusionSet`

Resolved once per workspace at `workspace/register` and stored on the registered workspace, not on
the watcher (research.md, *Where the exclusion set lives*). FR-006, FR-007, FR-008, FR-009.

| Field | Type | Rule |
|---|---|---|
| `builtin` | fixed list | `.git/`, `node_modules/`, `target/`, `dist/`, `build/`, `.venv/`, `__pycache__/` — seven entries, from §10.3, not configurable (FR-009) |
| `rules` | ordered, per-base-directory | The repository's own `.gitignore` files, resolved from disk at registration |
| `computed_at` | `Timestamp` | Registration. **Not** recomputed when a `.gitignore` is edited during a session |

`rules` is an *ordered* structure rather than a flat set of paths because `.gitignore` semantics
are per-directory, order-sensitive and admit negation with `!`. **Whether negation is honoured,
and with what precedence, is undetermined**: §10.3 says "the repository's own `.gitignore` files"
and stops, A-IGNORE declines configuration without describing matching, and research.md does not
address it. The matcher's exact semantics belong in contracts or design, and are not invented
here.

**Placement is the point.** Storing the set on the registered workspace makes FR-007's sharing
structural rather than conventional: the watcher reads it from there now, the indexer reads it
from there when F013 exists, and there is no second set for them to disagree about. The engine's
registry row, which F003 defined as `WorkspaceId → canonical AbsPath`, widens to:

| Entity | Shape | Lifetime |
|---|---|---|
| Registry row | `WorkspaceId → { root: AbsPath, exclusions: ExclusionSet, watched: WatchSet }` | The engine process |

**Lifetime and staleness.** The set lives as long as the registration. A `.gitignore` edited
mid-session takes effect on re-registration and not before — recorded in research.md as a decision
rather than discovered later as a bug.

---

## Client-side entities

### `Staleness`

A property of a **tree region**, meaning "re-query this before trusting it". FR-017, FR-026,
§10.4.

Persisted as `files.stale`, one flag per row, because the projection has no region table and a
region is expressed as the set of rows under a prefix (research.md, *Representing unproven
content*).

| Rule | Source |
|---|---|
| Set for every row of the workspace on `workspace/invalidateAll` | FR-017, §10.4 |
| Set for every row of the workspace on reconnection | FR-026 |
| Cleared for the rows one listing covers, when the developer navigates and `put_listing` replaces that folder's children | FR-017, §10.1 |
| Never triggers a fetch, a listing or a refetch by itself | FR-017, FR-020, SC-012a |
| Never deletes a row and never touches `file_contents` | FR-018, SC-006 |
| A stale row is still rendered and still navigable | FR-027, §10.4 |

**It is not content validity and not a `Presentation`.** §10.4 and §5.3 keep the two apart on
purpose: a blob under a stale region remains valid or not on its own hash terms, and marking the
tree stale is not an opinion about any byte. `Validity` and `Presentation` in
`client/core/src/domain/cache.rs` are untouched by this feature.

The asymmetry between setting and clearing is deliberate and is what makes staleness cheap: it is
marked wholesale and cleared one folder at a time, so a workspace marked stale returns to fresh
only as far as the developer walks and may never fully return. That is the intended behaviour, not
a leak.

**One consequence carried from the schema.** Because `files_fts_update` is declared
`AFTER UPDATE ON files` with no column list, an update that sets only `stale` fires the FTS5
delete-and-reinsert pair for terms that did not change. Marking a whole workspace stale is
therefore a full rewrite of that workspace's FTS index. See *Schema* below.

### `UnprovenContent`

A flag beside validity, **never a validity state** (A-UNPROVEN, FR-019, FR-019a, FR-019b).

Persisted as `file_contents.unproven`.

| Rule | Source |
|---|---|
| Set when a `FileEvent` names a path whose `files` row has a `file_contents` row | FR-019a |
| Set on a path with no cached content is a no-op — nothing is created and nothing is fetched | FR-020 |
| **Never** deletes the blob | FR-019a, SC-006a |
| Cleared when a hash comparison runs for that file, whatever the comparison's outcome | research.md, *Representing unproven content* |
| Marking an already-unproven row again changes nothing and causes no second fetch | spec Edge Cases, SC-006a |
| The blob remains servable while disconnected, presented as possibly stale | FR-019b, SC-006b |

It adds **no** `Validity` variant, because `Validity::compare` has exactly one constructor and it
takes two hashes — that is FR-019's "nothing else invalidates it" expressed as a type, and a third
variant would reopen it. It adds **no** `Presentation` variant either: `Verifying` covers the
connected confirmation FR-019a defers to, `Current` covers its success, and `PossiblyStale` is
already what FR-019b describes. F003's two enums are unchanged.

The flag is a *hint that the existing hash check will disagree*. It buys one thing: it does not
add a round trip, because F003 already confirms cached content against the engine before serving
it while connected (SC-006a asserts zero extra confirmations).

### `OpenTab`

The unit of "currently viewing" for FR-023 and FR-024 — a tab, focused or not.

| Field | Type | Rule |
|---|---|---|
| `workspace` | `WorkspaceId` | |
| `path` | `RelPath` | The file the tab shows |
| `focused` | `bool` | Which tab has focus; **not** what decides reporting (FR-023) |
| `changed` | `bool` | The FR-023a marker, set by an arriving event naming this tab's path |

**Interest is counted, not booleaned.** FR-024a ends reporting when the **last** tab on a file
closes, so more than one tab may name one path and the reporting set is the paths with at least
one open tab. The client therefore holds a count per path, not a set of paths.

| Rule | Source |
|---|---|
| A change to a file with an open tab is reported on that tab | FR-023, SC-001a |
| The report must not change focus or disturb the focused tab | FR-023a, SC-001a |
| A change to a file with no open tab must not interrupt; a tree row updating in place is not an interruption | FR-024 |
| Closing the last tab stops reporting for that file | FR-024a, SC-001b |
| Reporting must not depend on the file's folder being expanded | FR-003c, SC-001b |

**The engine never learns about tabs.** It learns only that a directory must be watched. Each open
tab contributes `parent(path)` and that directory's ancestors to the set the client sends
(research.md, *Watch scope*), which is what makes FR-003c satisfiable with no tab concept on the
wire. Expanded folders and open tabs are two separate sets whose union, closed under ancestors, is
the `WatchSet`.

What clears `changed` — returning to the tab, reloading it, or an explicit dismissal — is
**undetermined**. FR-023a requires only that the marker be visible when the developer returns and
that it not interrupt the focused tab; nothing states the clearing gesture, and this document does
not invent one.

---

## Schema: version 1 → version 2

`PRAGMA user_version` goes **1 → 2**. `schema::CURRENT_VERSION` becomes `2`, and `migrate`'s
`match step` gains a `2 => schema::V2` arm beside the existing `1 => schema::V1`.

```sql
ALTER TABLE files         ADD COLUMN stale    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE file_contents ADD COLUMN unproven INTEGER NOT NULL DEFAULT 0;

DROP TRIGGER files_fts_update;
CREATE TRIGGER files_fts_update AFTER UPDATE OF relative_path, name ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;
```

Two columns and one trigger replaced. The trigger is the part that would have been missed.

F003 shipped `files_fts_update` as `AFTER UPDATE ON files`, with no `OF` clause, so **every**
update to a row fires a delete-and-reinsert pair against the FTS5 index — including an update that
touches only `stale`, changing no indexed term. FR-017 and FR-026 mark the **whole workspace**
stale, so without this the cheapest operation in the feature becomes 2N pointless FTS5 writes for N
rows. §5.2 was amended to narrow the trigger, and because V1 already exists in the field the
migration has to drop and recreate rather than simply define it. The body is unchanged; only the
`UPDATE OF` clause is new.

Adding the two columns does not by itself disturb any of the three triggers: `ADD COLUMN` fires no
trigger, the triggers name `old.rowid`, `new.rowid`, `relative_path` and `name` explicitly rather
than using `*`, and the external-content rowids are untouched. The narrowing is an optimisation
forced by a new access pattern, not a repair.

That is the whole of V2. Two columns, one trigger, no new table, no new index, no change to an
existing column.

`V2` must contain **only** these statements, not V1 followed by them: `migrate` runs each step's
DDL in sequence, so step 2 executes against a database on which step 1 has already run.

**Why `NOT NULL DEFAULT 0` and not nullable.** A NULL would be a third state — "we do not know
whether this row is stale" — and both flags exist precisely to be answerable. It is also what
makes the migration free: SQLite's `ADD COLUMN` with a constant default records the default in the
table header and rewrites no rows, so a projection of a hundred thousand files migrates in
constant time.

Existing rows therefore arrive at v2 not stale and proven. That is the only honest default
available at migration time, because the migration runs before any connection exists and cannot
know what moved on a host it has not spoken to. Nothing is lost by it: everything the projection
then serves is still governed by §5.3's hash comparison, which neither flag participates in.

### What happens to the three FTS5 triggers

All three are on `files` — `files_fts_insert` (AFTER INSERT), `files_fts_delete` (AFTER DELETE)
and `files_fts_update` (AFTER UPDATE) — and each names its columns explicitly: `new.rowid`,
`new.relative_path`, `new.name`, and the `old.` equivalents. None uses `NEW.*`, none enumerates
the table's full column list, and none is declared `AFTER UPDATE OF <columns>`.

**Adding the column does not break them, and this is checkable rather than hopeful.** A trigger
body that refers to named columns is unaffected by a column appearing beside them; `ADD COLUMN`
fires no triggers, so the migration itself writes nothing to `files_fts`; and `files_fts` is
external-content keyed on `content_rowid='rowid'`, which `ADD COLUMN` does not change. No trigger
needs dropping and recreating, and no `rebuild` of the index is required.

**What does change is the cost of writing to the new column.** Because `files_fts_update` has no
`OF` clause, *every* UPDATE on a `files` row fires it, including one that touches only `stale`.
Each firing writes one FTS5 `'delete'` command row and one insert, re-indexing a `relative_path`
and a `name` that did not change. Marking a whole workspace stale under FR-017 or FR-026 is
therefore `2N` FTS5 writes for `N` rows, none of which alters a term.

research.md attaches a measurement obligation to the subtree rename for this reason
(*Directory rename with a subtree*). The whole-tree stale marking is the larger case and inherits
it. Narrowing the trigger to `AFTER UPDATE OF relative_path, name ON files` would remove the
amplification entirely — but that edits a v1 trigger that §5.2 now carries, which is a schema
decision this document records rather than takes.

`file_contents` has no triggers, so `unproven` carries no equivalent cost.

### First use of F003's migration machinery

F003 built the ladder and shipped it with one rung. `migrate` applies each step as **one
transaction that also sets `user_version`**, so a v1 → v2 that fails for any reason leaves a v1
file, untouched, and the next launch retries the same step; there is no moment at which a
half-migrated projection exists to be observed. A v2 file opened by a v1 build takes the
`FromTheFuture` branch and is rebuilt rather than read.

None of F003's migration tests crossed a version boundary, because there was none to cross. v2 is
the first real step, which makes the upgrade test — a populated v1 file, migrated, its rows intact,
its FTS index still answering and its two new columns reading 0 — as much the point as the columns
themselves (research.md, *Representing unproven content*).

---

## The subtree rename rewrite

FR-021, FR-022, and research.md, *Directory rename with a subtree*. The engine sends **one**
`renamed` event naming the directory; the client rewrites every descendant path in its projection.

### The separator-boundary hazard

Paths are stored in the normalised form F003 fixed: a leading `/`, `/` separators, no trailing
separator. The directory `src` is the stored value `/src`.

Matching `relative_path LIKE '/src%'` therefore also matches `/src-generated`, `/srcs` and
`/src.bak` — every sibling whose name merely begins with the same characters — and rewrites them
into paths that do not exist on the host. Nothing detects it: the rows stay well-formed, the
`UNIQUE` constraint is satisfied, and the tree quietly shows a subtree that is not there.

The correct match is the exact row **plus** the separator-terminated prefix, and nothing else:

```sql
WHERE workspace_id = :ws
  AND (relative_path = :from OR relative_path LIKE :from || '/%')
```

This is a test before it is an implementation: a fixture holding `/src` and `/src-generated`, a
rename of `/src`, and an assertion that `/src-generated` is untouched.

### Shape of the statement

```sql
UPDATE files
   SET relative_path = :to || substr(relative_path, length(:from) + 1),
       parent_path   = CASE WHEN relative_path = :from
                            THEN :to_parent
                            ELSE :to || substr(parent_path, length(:from) + 1) END,
       name          = CASE WHEN relative_path = :from THEN :to_name ELSE name END
 WHERE workspace_id = :ws
   AND (relative_path = :from OR relative_path LIKE :from || '/%');
```

Four things about that shape are load-bearing.

**Both path columns carry the prefix.** `relative_path` and `parent_path` both move; `name` changes
only for the renamed directory itself, because a descendant keeps its own name.

**The renamed directory's own `parent_path` does not carry the old prefix.** For a descendant,
`parent_path` is `/src/a` and the substring arithmetic is right. For the row being renamed,
`parent_path` is `/` or `/lib` — a path that has no `/src` prefix — so the same formula would set
the directory's parent to itself. That one row needs `:to_parent`, derived from `:to` by the
caller, which is why the `CASE` is not cosmetic.

**The `SET` expressions read the pre-update row.** `relative_path = :from` inside the `CASE` tests
the old value, which is what makes a single statement able to distinguish the renamed row from its
descendants.

**`file_id` is never touched.** That is what carries `file_contents` across the rename unchanged,
which is FR-021 and F003's guarantee C9 satisfied by the schema rather than by code remembering to
copy a blob.

### One transaction

The rewrite is one transaction, whether it is one statement or several. A partially renamed
subtree is a projection that disagrees with itself, which is worse than a stale one
(research.md). Two constraints follow.

`UNIQUE(workspace_id, relative_path)` is checked per row as the statement proceeds, so if the
projection already holds a row at the destination — a stale entry from an earlier listing — the
statement aborts and the transaction rolls back, leaving the subtree exactly as it was. That is
the right failure, but what the client then does (drop the colliding rows first, or fall back to
marking the region stale under FR-017) is **undetermined** and belongs in design.

`LIKE` will not use `idx_files_lookup` under this schema's settings. SQLite's LIKE optimisation
requires either `case_sensitive_like` on with a BINARY-collated column or a NOCASE column;
`apply_pragmas` sets `journal_mode`, `synchronous` and `foreign_keys` and nothing else, so
`case_sensitive_like` is off and `relative_path` is BINARY, and the prefix match scans the
workspace's rows. Turning the pragma on is a per-connection change that would alter the semantics
of every `LIKE` in the codebase, including `search_paths`; an explicit range
(`relative_path >= :from || '/' AND relative_path < :from || '0'`, `0` being the byte after `/`)
is the contained alternative. Which form is used is design's call; the separator-boundary rule is
identical in both.

### Trigger cost

The rewrite fires `files_fts_update` **once per affected row**, each firing being a `'delete'`
command row plus an insert — `2N` FTS5 writes inside the one transaction. Unlike the stale-marking
case, this work is not wasted: `relative_path` and `name` genuinely changed and the index must
follow. research.md requires that a large subtree rename be measured rather than assumed, and
names the fallback if it exceeds the budget — mark the subtree stale and rewrite lazily.

---

## Wire types

For `protocol/src/wire.rs`.

**On the naming convention, checked rather than repeated.** §4.8 states it in its own words:
"Field names in the tables above are written camelCase for readability; the wire carries
snake_case", giving `workspaceId` → `workspace_id`, `relativePath` → `relative_path`,
`nextCursor` → `next_cursor`. `wire.rs` implements it by doing nothing special: no params or
result struct carries `#[serde(rename_all)]`, the Rust field names are already snake_case, and the
only renames present are `kind` → `type` (a Rust keyword collision) and `EntryKind`'s lowercase
variants. F002 established the convention with `clientVersion` → `client_version`; §4.8 now says
so. So the framing is correct and it is a documented rule, not an accident — F004's new types
follow it by naming their fields snake_case and adding no attribute.

```rust
// ---- workspace/watch ----
pub struct WatchParams   { workspace_id: WorkspaceId, paths: Vec<String> }
pub struct WatchResult   { watching: u32, refused: Vec<WatchRefusal> }
pub struct WatchRefusal  { path: String, reason: RefusalReason }
pub enum   RefusalReason { CapacityExhausted, NotADirectory, NotFound }   // lowercase on the wire

// ---- workspace/unwatch ----
pub struct UnwatchParams { workspace_id: WorkspaceId, paths: Vec<String> }
pub struct UnwatchResult { watching: u32 }

// ---- workspace/onFileEvent ----
pub struct FileEventParams {
    workspace_id:  WorkspaceId,
    event:         FileEventKind,
    relative_path: String,
    to_path:       Option<String>,   // skip_serializing_if = "Option::is_none"
}
pub enum FileEventKind { Created, Modified, Deleted, Renamed }            // lowercase on the wire

// ---- workspace/invalidateAll ----
pub struct InvalidateAllParams { workspace_id: WorkspaceId }
```

**Paths are `String`, not `RelPath`.** This matches `RegisterParams::path`, which the existing
code comments as "untrusted, like every path off the wire", and it is what makes FR-014
satisfiable: a wire type that validated on deserialisation would turn a malformed path into a
parse error, and SC-010 asserts the client **refuses** such a path — which requires first
receiving it.

`to_path` is omitted when absent, as `cursor` and `next_cursor` already are, and is present exactly
when `event` is `renamed`. That is FR-011 expressed as a shape.

**`watching` is stated as a count above, and that is not settled.** research.md writes the result
as `{watching, refused[]}` without saying whether `watching` is a cardinality or the set itself.
The constraint it must meet is FR-025's: the client must be able to discover that it is not being
told about changes. A count detects disagreement cheaply; the full set detects *which* path
disagrees. **Undetermined — contracts/watch-methods.md settles it.**

Two further points the contracts must close.

**Whether a containment failure fails the call.** §4.7 requires the engine to reject a path that
escapes the root with `-32002`, which is unambiguous for a single-path method. With a list, one
bad path could fail the whole call or appear as a `refused[]` entry, and research.md's `refused[]`
is introduced for exhausted capacity, not for path escapes. Undetermined; stated rather than
guessed.

**Whether `onFileEvent` can carry a batch.** §4.8 defines the notification with a singular
`relativePath`, and research.md's list of required §4.8 amendments names only the two new method
rows and the narrowing of §10.3. But research.md's bulk-threshold derivation reasons about "256 of
them [at] about 64 KiB — a factor of eight inside the smaller limit", and plan.md's Constraints
say "the bulk threshold is chosen so an event batch cannot approach" the 1 MiB frame cap. Both
sentences describe several events sharing one frame, which the defined payload cannot do. Either
the notification is sent up to 256 times per bulk window, each in its own frame — in which case
the 64 KiB figure is not a frame size and the threshold's upper bound rests on nothing — or
`onFileEvent` gains an `events[]` form, which is a third §4.8 amendment nobody has recorded. **The
types above match §4.8 as it stands; the discrepancy is flagged, not resolved.**

**Neither addition increments `protocolVersion`.** §4.8 is explicit that adding a method does not.
What the client needs instead is a way to know the engine has them, and that is what
`auth/handshake`'s `capabilities` is for: capability tokens are method names, matched exactly and
never by prefix. An engine without `workspace/watch` in its set is an engine that cannot watch,
which is FR-005's report arriving before the first attempt rather than after it.

---

## State transitions

### A cache entry, as events act on it

Two axes move independently, which is the whole point: the `files` row's existence and path, and
the `file_contents` row's `unproven` flag. **No event moves `Validity`**, which remains a
comparison of two hashes and nothing else (FR-019, §5.3).

| Event | Client holds no row | Tree row, not cached | Tree row with cached content |
|---|---|---|---|
| `created` | nothing happens; not fetched (FR-020) | the row exists already | — |
| `modified` | nothing happens (FR-020) | nothing to mark; there is no blob | `unproven = 1`, blob kept (FR-019a) |
| `deleted` | nothing happens | row deleted; the FTS delete trigger fires | row deleted, `file_contents` cascades (F003 invariant 1) |
| `renamed` | nothing happens | path columns updated; `file_id` unchanged | same, and the blob survives the move (FR-021, FR-022) |

`invalidateAll` and reconnection do not appear in that table because they touch neither axis for a
*content* row: they set `files.stale` and leave `file_contents` entirely alone (FR-018, FR-026a,
SC-006, SC-012a).

**One case has no stated resolution.** A `created` event for a file inside a folder the developer
has expanded must make the file appear in the tree (US1 acceptance 1), but the event carries no
size, no modification time and no kind (FR-013), and `files` declares `size_bytes`,
`remote_modified_at` and `is_directory` all `NOT NULL`. So the row cannot be inserted from the
event alone. The three candidates are a `workspace/stat` for the new path, a re-listing of the
parent folder, or a row with sentinel metadata. FR-020 forbids *fetching* a path the client has
never fetched, and the reading that reconciles it with US1 is that "fetched" means content, while
a new child inside a listing the client already holds is an update to that listing rather than a
new path being fetched. **That is a reading, not a stated rule**, and research.md resolves none of
it. Recorded here so it is decided in design rather than discovered in implementation.

### A tree region's staleness

```
Fresh ──workspace/invalidateAll (FR-017)──▶ Stale
Fresh ──reconnection (FR-026)────────────▶ Stale
Stale ──developer navigates; put_listing replaces that folder's children──▶ Fresh (that folder only)
Stale ──disconnection────────────────────▶ Stale   (nothing resolves it while offline)
```

Marked at workspace scope, cleared at folder scope. A stale region stays rendered, stays navigable
and issues no request of its own (FR-017, FR-027, SC-012a); it re-reads only where the developer
walks, which is §10.1's approach applied to correction rather than to first load.

---

## Invariants

Things a test should be able to break and find something wrong.

| # | Invariant | Enforced by | Requirement |
|---|---|---|---|
| 1 | No event is ever delivered for an excluded path | Excluded directories are never watched, and excluded raw events are dropped before coalescing | FR-008, SC-002 |
| 2 | The watcher's exclusion set and the indexer's are the same object, not two equal ones | One set on the registered workspace | FR-007, SC-003 |
| 3 | `to_path` is present exactly when the event kind is `renamed` | Wire type shape | FR-011, SC-008 |
| 4 | At most one event per path per coalescing window | The coalescer, over a fake clock | FR-012, SC-007 |
| 5 | More than 256 distinct paths in one second yields exactly one `invalidateAll` and zero individual events | The coalescer, over a fake clock | FR-015, SC-004 |
| 6 | No event carries content | Wire type has no content field | FR-013 |
| 7 | An arriving path that escapes the root is refused client-side, including one the engine sent | Client containment, run before any projection write | FR-014, SC-010 |
| 8 | `invalidateAll` and reconnection delete zero rows from `file_contents` | The statement only writes `files.stale` | FR-018, FR-026a, SC-006, SC-012a |
| 9 | No event path constructs a `Validity` | `Validity::compare` takes two hashes and nothing else | FR-019, A-UNPROVEN |
| 10 | Marking an already-unproven row again changes nothing and causes no second confirmation | Idempotent UPDATE; the confirmation is F003's existing one | spec Edge Cases, SC-006a |
| 11 | A subtree rename leaves sibling prefixes untouched | The separator-boundary match | FR-022 |
| 12 | A subtree rename preserves every `file_id`, so cached content survives | The statement never writes `file_id` | FR-021, SC-008, F003 C9 |
| 13 | The `WatchSet` never contains a path whose watch was refused | Refusals are reported and not added | FR-005, FR-025 |
| 14 | Closing a workspace returns the `WatchSet` to empty | Release on close, drop and exit | FR-004, SC-009 |
| 15 | The watch count is proportional to what is expanded and open, never to repository size | The client computes the set; the engine watches only what it is told | FR-003, SC-009a |
| 16 | A v1 projection migrates to v2 with every row intact and the FTS index still answering | One transaction per step, and the upgrade test | research.md; FR-018b (F003) |

Invariant 2 deserves the same note F003 gave its own third: it is asserted by comparing the two
sets rather than by asserting that one function was called, because SC-003 says "compared as sets,
not asserted" and because the failure it guards against — an indexer quietly computing its own —
is invisible to any test that only checks the watcher.
