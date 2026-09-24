# Contract: File Event Notifications

**Feature**: F004 file-watch-sync | **Date**: 2026-09-24

The two §4.8 notifications this feature implements. Both already exist in the catalogue as rows;
neither has ever been specified beyond its parameter names, and this document is where the
guarantees behind them are written down. It also marks the one place the catalogue must be
amended.

Notifications carry no `id` and expect no response (§4.2). They are engine-originated: nothing in
`dispatch` produces them, and nothing correlates them.

The values below — the 100 ms window, the 256-path threshold, the single directory-rename event —
are fixed in research.md under *The coalescing window*, *The bulk threshold*, *Rename detection*
and *Directory rename with a subtree*. Rationale lives there and is not restated.

---

## `workspace/onFileEvent`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `events` | array of event objects, never empty | yes |
| event | `event` | one of `created`, `modified`, `deleted`, `renamed` | yes |
| event | `relativePath` | string, workspace-relative | yes |
| event | `toPath` | string, workspace-relative | **exactly when** `event` is `renamed` |
| event | `type` | `file` or `directory` | on `created` and `modified` |
| event | `size` | integer, bytes | on `created` and `modified`, for a file |
| event | `modified` | integer, Unix seconds | on `created` and `modified` |

**The notification carries an array.** One flush of the coalescer is one frame: everything whose
window closed at the same instant travels together. §4.6 makes this one pipe and one queue, and a
burst delivered as hundreds of separate frames would take the writer hundreds of times ahead of
whatever interactive request is queued behind it (FR-016). It is also what makes A-COALESCE's
upper bound on the 256-path threshold a real frame-size constraint rather than an aggregate across
frames that never approach the cap.

`type`, `size` and `modified` are the same entry metadata `workspace/readDirectory` returns. **In the same units**: `modified` is
Unix seconds, as `FsEntryWire` already uses. Found during implementation, where the first draft
wrote milliseconds — which would have made "the same metadata a listing returns" false in the one
way a reader would not check. They
are present because without them a `created` event cannot produce a row: `files` declares
`size_bytes`, `remote_modified_at` and `is_directory` all `NOT NULL`, so US1's first acceptance
scenario would require a follow-up `stat` that FR-020 forbids. This is metadata, not content —
FR-013 forbids bytes and a hash; FR-013a authorises exactly this metadata, and none of it tells
the client what a file now says.

Wire spelling is snake_case (§4.8): `workspace_id`, `events`, `event`, `relative_path`, `to_path`,
`type`, `size`, `modified`. The `event` values are single lowercase words on the wire, matching
`EntryKind`'s precedent in `protocol/src/wire.rs`.

**AMENDED §4.8, 2026-09-24.** The catalogue named the `event` parameter and never said what may be
in it, carried a singular `relativePath`, and had no metadata. All three are now fixed in the
catalogue. The four kinds above are the complete vocabulary, and §4.8 states them: a third party — or a
second implementation of this engine — reading the catalogue alone has no way to know whether
`renamed` exists, which is the one kind FR-011 makes mandatory. Adding a description of an
existing parameter does not increment `protocolVersion` (§4.8).

### The four kinds

| Kind | Means | `toPath` |
|---|---|---|
| `created` | A path that was not there is there now, including one moved in from outside the workspace | absent |
| `modified` | An existing file's content or metadata changed | absent |
| `deleted` | A path that was there is gone, including one moved out of the workspace | absent |
| `renamed` | One path became another **within** the workspace | **present** |

`created` and `deleted` cover the unpaired halves of a move across the workspace boundary. That
is not a degradation of rename detection, it is the correct classification: a file moved out of
the workspace genuinely is a deletion from this workspace's point of view (research.md, *Rename
detection*).

### Guarantees

1. **An event carries no content, ever** (FR-013). Not the bytes, not a hash, not a diff. It says
   something changed, not what the file now *says*. The absence of a content field is what
   enforces FR-019 structurally rather than by rule — a client cannot mark a blob valid from an
   event, because an event contains nothing a validity decision could be made from. **A hash is
   the line**: `size` and `modified` are present (FR-013a) and neither can settle validity, which
   is why carrying them costs nothing structurally. Were a hash ever added here, FR-019 would stop
   being structural and become a rule somebody has to keep.
2. **A rename is one event naming both paths** (FR-011), never a `deleted` followed by an
   unrelated `created`. F003's projection survives a rename only if the entry moves rather than
   being lost and re-found (its FR-022 and guarantee C9), and a client cannot reconstruct the
   pairing from two events without hashing content it does not have.
3. **A directory rename is ONE event naming the directory** (FR-022, research.md, *Directory
   rename with a subtree*). Descendants are not enumerated. Emitting one event per descendant
   would produce the flood the bulk threshold exists to prevent, caused by the feature meant to
   prevent it. The client rewrites the subtree itself; the obligation is stated below.
4. **Repeated changes to one path collapse into at most one event per 100 ms**, flushed on the
   **trailing** edge (FR-012). The bound is on elapsed time, not on writes: **at most ten events
   per second per path**, whatever the writer does. This is the number SC-007 asserts against,
   and a test reads it from the plan rather than assuming it.
5. **The trailing edge is contractual, not an implementation detail.** The last write in a burst
   is the one whose content matters. A leading-edge flush would report the first write and
   debounce away the state the developer would actually fetch (research.md, *The coalescing
   window*).
6. **No event is delivered for an excluded path** (FR-008, SC-002), for any kind, whether or not
   the developer has expanded the directory containing it. The exclusion set is the one computed
   per workspace at registration and shared with the indexer (FR-006, FR-007, A-IGNORE,
   research.md, *Where the exclusion set lives*).
7. **No event is delivered for a path outside the workspace root** (FR-002). The client
   re-validates this for itself regardless — guarantee 13.
8. **Events for one path arrive in the order they occurred.** Across paths, no ordering is
   promised beyond frame order on the one pipe (§4.6): two paths whose windows expire in the same
   flush may be written in either order, and no client behaviour may depend on which.
9. **Event delivery does not delay interactive traffic** (FR-016, §4.6, Principle V). The
   watcher thread holds the output mutex for at most one frame at a time (research.md, *A watcher
   in a runtime-free engine*). This is a measurement obligation under Principle V, not a comment.
10. **An `invalidateAll` supersedes the individual events in the same flush.** When the bulk rule
    fires, the pending batch is discarded rather than sent alongside it — SC-004 requires exactly
    one wholesale invalidation and **zero** individual events.

### What is delivered: the requested set is the filter

The host watch set is larger than the requested set (watch-methods.md, `workspace/watch`
guarantee 4). Ancestors and the root are watched to observe ancestor renames and the root's
disappearance, not to report their other children. An event for path `P` is delivered exactly
when, at flush time, one of these holds and `P` is not excluded:

- `P` is in the requested set — a requested file, or a requested directory itself changing
  (FR-003c for files, FR-003 for directories);
- `P`'s parent directory is in the requested set — the expanded-folder case (FR-003, FR-003a);
- the event is a `renamed` whose `relativePath` is an ancestor of some path in the requested set —
  the case FR-022 exists for, and the reason ancestors are watched at all.

Anything else observed by an ancestor watch is dropped in the engine, before coalescing. This is
US1 acceptance scenario 4 stated as a rule: **a folder the developer has never expanded produces
no event and fetches nothing**, even though its parent may be watched.

---

## `workspace/invalidateAll`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |

No paths, no count, no cause. There is deliberately nothing else: a client that could read *why*
would be tempted to act differently for different causes, and §10.4 specifies one response.

### Guarantees

1. **Fired when 256 distinct paths change within a rolling 1-second window** (FR-015, SC-004,
   research.md, *The bulk threshold*). The bulk window is deliberately **not** the coalescing
   window: coalescing is per path over 100 ms; the bulk count is across paths over 1 second.
2. **Exactly one is sent per burst, and the individual events are discarded**, not queued behind
   it (guarantee 10 above, SC-004).
3. **It invalidates the tree, never content** (FR-018, §10.4, §5.3). Zero cached blobs are
   discarded — SC-006 asserts the number. Guarantee 5 is not a counterexample: marking an open
   tab's content **unproven** is not invalidating it. The blob stays, stays servable offline, and
   the hash still decides. Discarding and doubting are different operations, which is the whole
   reason `unproven` is a flag beside validity rather than a validity state (A-UNPROVEN).
4. **It is not a fetch instruction** (FR-017). The client marks the tree stale and re-queries
   lazily as the developer navigates; SC-012a asserts zero listing requests until they do.
5. **Every open tab is marked unproven by it** (FR-023b, SC-004a). The bulk rule discards the
   individual events, so a branch switch that rewrites a file the developer has open would
   otherwise report nothing about that file, and FR-023 admits no exception for how the change
   arrived. The **client** does this from its own tab list; nothing extra crosses the wire, and
   the engine is not asked to exempt open tabs from the bulk rule.
6. **The engine never sends it on reconnection.** It cannot distinguish a reconnecting client
   from a connecting one. FR-026 requires the *client* to apply this same handling locally on
   reconnection, unprompted. A client waiting for a notification that is never sent shows a tree
   it believes is fresh.

---

## The client's obligations

These are guarantees the client makes, not the engine. They are in this contract because they are
the receiving half of it, and because Principle VI makes the receiving end responsible regardless
of what the sending end did.

11. **Every arriving path is untrusted, and containment is re-checked** (FR-014, SC-010,
    Principle VI). A path escaping the workspace root is refused and dropped **in 100% of cases,
    including when the engine sent it** — a symlinked directory or a malformed event is exactly
    the case, and F002's threat model makes this defence against a stale or buggy engine rather
    than a hostile one. Both `relativePath` and `toPath` are checked; a rename with a valid source
    and an escaping destination is refused whole.
12. **An event for a path the client no longer watches is dropped.** The engine stops observing
    when `unwatch` returns; it does not recall frames already written (watch-methods.md,
    `workspace/unwatch` guarantee 5). The spec's *folder collapsed while its files are changing*
    edge case is this rule. The test is the client's **current requested set**, not its expanded
    folders — a collapsed folder still holding an open tab is still watched (FR-003c).
13. **An event for a path the client has never fetched fetches nothing** (FR-020). There is no
    tree entry to update and no blob to mark, and fetching would defeat §10.1.
14. **An event NEVER marks content valid** (FR-019). Validity is a hash comparison and nothing
    else (§5.3). This is the single most important thing a client may not infer from an event, and
    the reason is structural: F003 built `Validity` with exactly one constructor taking two
    hashes so that no path exists by which anything other than a hash comparison can declare
    content valid (research.md, *Representing unproven content*).
15. **An event naming a cached file marks that blob unproven and does not delete it**
    (FR-019a). Unproven is a flag beside validity, never a validity state. The next read confirms
    it against the engine before serving — which is what F003 already does for every cached read
    while connected, so the event adds **no second mechanism and no second round trip**
    (SC-006a asserts zero extra confirmations).
16. **Unproven content stays servable while disconnected**, presented as possibly stale
    (FR-019b, SC-006b). An event received seconds before an outage must not cost the developer a
    file they had, for a change they may never open.
17. **Marking an already-unproven blob changes nothing.** The hash already disagrees and the file
    is already unproven; a second event causes no second fetch (spec edge case, SC-006a).
18. **A rename moves the tree entry rather than removing and re-adding it** (FR-021), so cached
    content survives the move (F003's FR-022 and its `rename` cache operation, whose first caller
    this is).
19. **A directory rename rewrites every descendant path, bounded to a separator** (FR-022). The
    match is the exact row `relative_path = 'src'` **together with** `relative_path LIKE 'src/%'`.
    A bare `LIKE 'src%'` also rewrites `src-generated`, silently corrupting unrelated rows — and
    that is a test before it is an implementation (research.md, *Directory rename with a
    subtree*).
20. **The subtree rewrite is one transaction.** A partially renamed subtree is a projection that
    disagrees with itself, which is worse than a stale one. It fires F003's three FTS5
    synchronisation triggers once per affected row, so a large subtree rename is a bulk trigger
    run and is measured rather than assumed (Principle V, research.md same section).
21. **A change to a file with an open tab is reported on that tab** (FR-023, FR-023a). Focused or
    not; the report attaches to the tab it concerns, does not move focus, and does not interrupt
    the tab being worked in (SC-001a).
22. **A change to a file with no open tab does not interrupt the developer** (FR-024). A tree row
    updating in place is not an interruption; a prompt, a dialog or a shift of focus is.
23. **While events are not being reported — disconnected, or watching refused — the developer is
    told** (FR-025, FR-005, SC-011). Silence is not how a developer discovers that watching
    failed, and a refusal in `watch`'s `refused[]` is a report the client owes onward.

---

## Worked examples

Frames are snake_case and length-prefixed per §4.1; headers omitted. Notifications carry no `id`.

**A file created in an expanded folder** (US1 scenario 1).

```json
{"jsonrpc":"2.0","method":"workspace/onFileEvent",
 "params":{"workspace_id":"ws_7f2a","events":[
   {"event":"created","relative_path":"src/parser/token.rs",
    "type":"file","size":2048,"modified":1758700000000}]}}
```

**A file modified — one event, however many writes.** A compiler writes `out/app.wasm` four
hundred times in a second; the coalescer delivers at most ten (guarantee 4, SC-007).

```json
{"jsonrpc":"2.0","method":"workspace/onFileEvent",
 "params":{"workspace_id":"ws_7f2a","events":[
   {"event":"modified","relative_path":"src/parser/expr.rs",
    "type":"file","size":8192,"modified":1758700001000}]}}
```

The client marks the blob unproven and does not delete it (obligation 15). It does **not** mark
it valid, and it does not fetch (obligations 14, 13). If `src/parser/expr.rs` has an open tab, the
tab is marked changed without taking focus (obligation 21).

**A file rename — one event, both paths** (FR-011, SC-008).

```json
{"jsonrpc":"2.0","method":"workspace/onFileEvent",
 "params":{"workspace_id":"ws_7f2a","events":[
   {"event":"renamed","relative_path":"src/parser/expr.rs",
    "to_path":"src/parser/expression.rs"}]}}
```

The client moves the tree entry (obligation 18). The cached content survives, because `file_id`
is preserved across a *known* move — which is the case F003's `rename` operation was built for
and this is its first caller.

**A directory rename — still one event** (FR-022).

```json
{"jsonrpc":"2.0","method":"workspace/onFileEvent",
 "params":{"workspace_id":"ws_7f2a","events":[
   {"event":"renamed","relative_path":"src/parser","to_path":"src/syntax"}]}}
```

Eleven hundred descendants changed path and **none of them was individually touched**. The
engine sends nothing further. The client rewrites `src/parser` and every `src/parser/%` row to
the new prefix, in one transaction, bounded to the separator so that `src/parser-old` is
untouched (obligations 19, 20).

**A file moved out of the workspace** — the unpaired half, classified as a deletion
(research.md, *Rename detection*).

```json
{"jsonrpc":"2.0","method":"workspace/onFileEvent",
 "params":{"workspace_id":"ws_7f2a","events":[
   {"event":"deleted","relative_path":"src/parser/scratch.rs"}]}}
```

**A branch switch** — nine thousand paths changed inside one second (US2, SC-004).

```json
{"jsonrpc":"2.0","method":"workspace/invalidateAll",
 "params":{"workspace_id":"ws_7f2a"}}
```

That is the **entire** delivery. Zero `onFileEvent` frames accompany it (guarantee 10). The tree
is marked stale and re-read lazily (FR-017, guarantee 4 of `invalidateAll`); zero cached blobs are
discarded (FR-018, SC-006); interactive actions continue to meet §1.4 throughout, measured and
printed (SC-005, A-NFR).

**An event the client refuses** (FR-014, SC-010). The engine should never send this — FR-002
forbids it and §4.7 makes containment the engine's job — and the client refuses it anyway,
because Principle VI makes the receiving end responsible for what the sending end claims to have
checked.

```json
{"jsonrpc":"2.0","method":"workspace/onFileEvent",
 "params":{"workspace_id":"ws_7f2a","events":[
   {"event":"modified","relative_path":"../../etc/shadow",
    "type":"file","size":1,"modified":1758700002000}]}}
```

Dropped. Nothing is marked, nothing is fetched, nothing is rendered, and the refusal is recorded
where a developer can find it — a client that silently discards it loses the one signal that says
the engine is stale or wrong.

---

## What a client may and may not infer

Stated as a table because every row of it is a way an implementation has gone wrong before.

| From an event, a client MAY infer | A client MUST NOT infer |
|---|---|
| That the path named may have changed on the host | That it *has* changed — a coalesced window can flush after a write that was reverted |
| That a cached blob for that path is **unproven** (FR-019a) | That a cached blob is **valid**, ever, for any kind (FR-019) |
| That a tree entry should move, for `renamed` (FR-021) | That it should be removed and re-added — cached content would not survive |
| That an open tab should be marked changed (FR-023) | That focus should move, or the focused tab be interrupted (FR-023a, FR-024) |
| That a tree region is stale, for `invalidateAll` (FR-017) | That cached content is stale — invalidation of the tree is not invalidation of content (FR-018, §10.4, §5.3) |
| That the developer should be told, when watching is refused (FR-005, FR-025) | That silence means stability — the engine may have restarted (spec edge case) |
| Entry metadata: type, size, modified (FR-013a) | Anything about content — bytes, or a hash. An event carries neither (FR-013) |

---

## Amendments this feature made to the system specification

**All applied 2026-09-24**, before implementation.

1. **§4.8 now states the `event` vocabulary** — `created`, `modified`, `deleted`, `renamed` — and
   that `toPath` is present exactly for `renamed`. The catalogue previously named the field and
   defined no values, which left FR-011's mandatory kind undiscoverable from the specification.
2. **A-COALESCE records the 100 ms window and the 256-path threshold.** §10.4 gave the cases and
   no number, and FR-012 and FR-015 both require the value to be stated rather than judged.
3. **`onFileEvent` now carries an array, and entry metadata.** This was recorded as unsettled when
   this contract was first written, and it is settled: §4.8's row is
   `events[]` of `{event, relativePath, toPath?, type?, size?, modified?}`.

   Two things forced it. The batch is better for FR-016 — one writer acquisition per flush rather
   than up to 256 interleaved with interactive traffic — and it makes A-COALESCE's frame-size
   upper bound on the threshold a real constraint instead of an aggregate across frames that never
   approach the cap. The metadata is what makes a `created` event able to produce a `files` row at
   all, since `size_bytes`, `remote_modified_at` and `is_directory` are `NOT NULL` and the
   alternative was a follow-up `stat` that FR-020 forbids.
