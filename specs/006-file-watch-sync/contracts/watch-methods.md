# Contract: Watch Methods

**Feature**: F004 file-watch-sync | **Date**: 2026-09-24

The two requests this feature adds to §4.8, on both ends. `project-apex-predator.md` §4.8 is the
source of truth for the method catalogue; this document states the guarantees the catalogue's
table has no room for, and marks the places it must be amended.

Every method takes `workspaceId`. Every path in `paths[]` is **untrusted input** at the engine
(§4.7, Principle VI), and every path in an event is untrusted again at the client
(FR-014, file-events.md).

Rationale for the shapes below is recorded in research.md, *Protocol additions* and
*Watch scope: what is actually watched*. It is not restated here.

---

## `workspace/watch` — **NEW METHOD, AMENDS §4.8**

**§4.8 does not define this method and it must be added before implementation.** The catalogue
carries `workspace/onFileEvent` and `workspace/invalidateAll` as notifications and **no request
to begin or end a watch**, while §6.1 declares `watch()` on the provider trait and §10.3
describes the engine watching from registration. FR-003b names the contradiction; the plan's
Constitution Check marks it blocking under Principle II; research.md resolves it and records
**A-WATCHSCOPE** as owed. This is the fifth absence of its kind, after `workspace/register`
(F003).

Adding a method does **not** increment `protocolVersion` (§4.8). An older engine answers
`-32601`, which is exactly the signal a client needs to know it must redeploy.

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `paths[]` | array of string, workspace-relative | yes |
| result | `watching` | integer | yes |
| result | `refused[]` | array of `{path, reason}` | yes — possibly empty |

`reason` is one of `capacity`, `excluded`, `not_found`, `not_a_directory`. The vocabulary is
closed, and a client MUST treat an unrecognised value as `capacity` — the conservative reading,
because every reason means the same thing to the developer: **this path is not being watched**
(FR-005).

**Wire spelling.** §4.8's tables are camelCase for readability and the wire carries snake_case.
`workspaceId` is `workspace_id`; `paths`, `watching`, `refused`, `path` and `reason` are already
single words. `relativePath` and `toPath` appear as `relative_path` and `to_path` in
file-events.md.

### Guarantees

1. **Set semantics, per workspace.** The engine holds one set of requested paths per workspace.
   `watch` is a union into that set; it is **idempotent**, and a path already present is neither
   an error nor a refusal (FR-003b). Watching the same path twice leaves `watching` unchanged.
2. **Not a reference count.** A path present once is present once. Two reasons for wanting a
   path watched — an expanded folder that also holds an open tab — do **not** produce two
   entries, and this is why the set holds the *thing the client asked for* rather than the
   reason it asked (guarantee 3).
3. **A path may be a directory or a file, and the distinction is contractual.** A directory in
   the set means "report changes to this directory's immediate children" — the expanded-folder
   case of FR-003 and FR-003a. A file in the set means "report changes to this file" — the
   open-tab case of FR-003c. The engine watches directories only, never files, and derives the
   host watch set from the requested set (research.md, *Watch scope*).

   **This is what makes FR-004 implementable without set arithmetic in the client.** Collapsing a
   folder sends `unwatch` for the folder path. If a file inside it is still open, that file's own
   path is still in the set, so the directory stays watched and reporting for the open tab
   survives the collapse — which is precisely what FR-003c requires and what a folder-only set
   would break. A client that instead sent `unwatch` for a directory and expected the engine to
   remember why would be relying on state it never sent.
4. **The host watch set is larger than the requested set, and the difference is invisible here.**
   Ancestors up to the root, and the root itself, are watched from registration and never
   released while the workspace is open (research.md, *Watch scope*). They are watched to observe
   ancestor renames (FR-022) and the root's disappearance (`-32009`), not to deliver events for
   their other children. Delivery is filtered by the **requested** set; the rule is stated in
   file-events.md, *What is delivered*.
5. **`watching` counts requested paths, not host watches.** It is the size of the requested set
   after the call, and it exists so a client can detect divergence from what it believes it
   asked for in one integer rather than by diffing a list it already holds. SC-009, SC-009a and
   SC-009b are assertions about **host** watch resources and are measured on the host; they are
   not read from this field, which by guarantee 4 is a smaller number.

   **Not fixed by research.md.** research.md writes the result as `{watching, refused[]}` without
   saying whether `watching` is a count or a list. A count is chosen here because returning the
   full set on every expand puts an O(set) payload inside the §1.4 interaction budget for an
   operation whose whole point is that it is small, and duplicates state the client authored.
   If a client is ever found to need the engine's view enumerated, that is a new optional result
   field and not a breaking change (§4.8).
6. **No call partially fails.** Every path given is either in the set afterwards or named in
   `refused[]` with a reason. There is no third outcome and no ordering in which a client learns
   nothing about a path it sent.
7. **`refused[]` preserves the order of `paths[]`**, so a client can correlate without matching
   strings. Duplicates within one call collapse before processing and are reported at most once.
8. **A refusal is not an error, and this is the whole reason the result has this shape.** FR-005
   requires the system to report when it cannot watch; FR-005a requires the workspace to stay
   open and browsable when it cannot. An error would satisfy the first and breach the second, so
   a partial result is the only shape that satisfies both (research.md, *Protocol additions*).
9. **An excluded path is refused, never silently accepted.** A path inside the resolved exclusion
   set is returned in `refused[]` with `reason: "excluded"` and is not added. FR-008 forbids
   delivering an event for it, and accepting the request would leave the client believing a
   directory is watched that will never produce an event — FR-005's silence, arriving through the
   success path.
10. **A path that does not exist is refused, not an error.** `reason: "not_found"`, and the call
    succeeds. See *Why `-32003` does not apply* below; this is the guarantee FR-026b depends on.
11. **Capacity exhaustion refuses the paths it could not take and keeps the rest.** The set
    contains everything that was accepted, `watching` reflects it, and the developer is told
    which freshness they lost (FR-005, FR-005a, FR-027, SC-009c).
12. **Dropping the connection empties the set.** FR-004 requires watches released when the
    connection drops, so a reconnecting client never inherits a stale set and never needs to
    reconcile one. This is what makes the reconnection contract below a single call.
13. **The set is in memory and dies with the engine**, like the workspace registry
    (§4.8, F003's `workspace/register`). A client re-attaching to a restarted engine registers
    first and watches second; `session/onRestart`'s `unpreserved` list is how it learns it must.

### Errors

Call-level errors. Each fails the whole call and changes nothing in the set.

| Code | Condition |
|---|---|
| `-32001` | Unknown or unregistered `workspaceId` (§4.4). The client's answer is to register and retry |
| `-32009` | Registered, and the root no longer exists (§4.4). The client's answer is to tell the developer and stop presenting its projection as a live view — **not** to re-register |
| `-32002` | A path in `paths[]` escapes the workspace root, lexically or after symlink resolution (§4.7) |
| `-32602` | `paths` is absent, not an array, or contains a non-string |

`-32003` is **not** returned by either method. See below.

#### Why `-32002` fails the whole call while every other per-path problem does not

An escaping path cannot be part of a legitimate desired set. The client builds `paths[]` from its
own projection, so an escape is a client bug or a hostile frame, and §4.7 is normative for every
method: the engine canonicalises and asserts containment, "rejecting anything else with
`-32002`". Degrading that to a per-path refusal would make the boundary check advisory — the one
thing Principle VI says it must never be. The cost is that one bad path fails a reconnection's
re-establishment, which is the correct cost: a client that cannot construct its own paths should
fail loudly rather than resume watching most of what it wanted.

#### Why `-32003` does not apply

§4.4 reserves `-32003` for a path inside the root that does not exist, and every F003 method
returns it. This method does not, deliberately.

FR-026b requires reconnection to re-establish watches for **everything** still expanded and open,
in one call carrying the current set. A folder deleted on the host while the client was
disconnected is exactly the case that call must survive. Returning `-32003` would fail the whole
re-establishment because of one path the client could not have known about, leaving every other
folder unwatched — FR-025's silent non-updating tree, reached by the route FR-026b exists to
close. A per-path `not_found` refusal tells the client the same fact and costs it nothing else.

---

## `workspace/unwatch` — **NEW METHOD, AMENDS §4.8**

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `paths[]` | array of string, workspace-relative | yes |
| result | `watching` | integer | yes |

### Guarantees

1. **Set difference, and idempotent.** Removing a path not in the set succeeds and changes
   nothing. There is no `refused[]`, because there is no per-path failure to report: a path the
   engine is not watching is already in the state the caller asked for.
2. **Removal is by the path given.** Unwatching a directory does not remove a file inside it that
   was requested separately, and unwatching a file does not remove its directory. Guarantee 3 of
   `watch` is what makes FR-004 fall out of this rather than needing a rule.
3. **Ancestors and the root are never released by this call.** They are engine-derived
   (guarantee 4 of `watch`) and the root is held for the life of the workspace. Releasing an
   ancestor because it was unwatched as a requested path would lose the rename observation that
   FR-022 depends on for every path still beneath it.
4. **Host watches are released when the last requested path needing them goes.** This is the
   guarantee SC-009b measures: collapsing a folder with no open tab inside it returns the count
   to its pre-expansion level; collapsing one that still holds an open tab releases zero watches
   that tab depends on.
5. **Events already in flight are not recalled.** The engine stops observing; it does not chase
   frames already written. The client's obligation to drop an event for a path it no longer
   watches is stated in file-events.md and is the spec's *folder collapsed while its files are
   changing* edge case.
6. **Closing a workspace needs no `unwatch`.** Watches are released when the workspace closes,
   when the connection drops and when the engine exits (FR-004), which is what SC-009's hundred
   open-and-close cycles assert against. `unwatch` exists for the collapse and tab-close cases,
   not for teardown.

### Errors

| Code | Condition |
|---|---|
| `-32001` | Unknown or unregistered `workspaceId` |
| `-32009` | Registered, and the root no longer exists |
| `-32002` | A path escapes the workspace root (§4.7) |
| `-32602` | `paths` is absent, not an array, or contains a non-string |

`-32003` does not apply, for a second reason beyond the one above: unwatching a path that has
since been deleted is the **normal** way a client tidies up after a deletion event, and an error
there would make the correct sequence look like a failure.

---

## The reconnection contract (FR-026b)

One `watch` call carrying the full current set re-establishes everything.

```
connection returns
  → auth/handshake            (resumed true or false; F002)
  → workspace/register        (the engine's registry did not survive; §4.8)
  → workspace/watch  paths = every folder still expanded
                           + every file still open in a tab      FR-003c, FR-026b
  → mark the whole tree stale, re-read lazily as the developer navigates
                                                                 FR-026, FR-026a
```

Three properties make that one call sufficient rather than a replay:

1. **The engine's set is empty.** Guarantee 12: the drop released it. The client is establishing,
   not reconciling, so no `unwatch` is needed for anything collapsed while disconnected.
2. **`watch` is idempotent** (guarantee 1), so a client that is wrong about whether the drop was
   observed is still correct after the call. A per-path API would make the client replay a
   remembered history, and a client that mis-remembers resumes believing it is being told about
   changes when it is not — FR-025's failure by another route (research.md, *Protocol additions*).
3. **Refusals arrive in the same result.** A folder deleted while disconnected comes back as
   `not_found` and a capacity ceiling comes back as `capacity`, both without failing the
   re-establishment (guarantees 10 and 11). SC-012b asserts that every folder still expanded and
   every file still open is watched afterwards; a refusal is how the client learns which ones
   are not, and FR-025 makes telling the developer mandatory rather than optional.

**The engine sends no `workspace/invalidateAll` on reconnection.** It has no way to know a client
reconnected rather than connected. FR-026 requires the client to apply the *same handling* it
applies to that notification, locally and unprompted. Stated here because a client waiting for a
notification that is never sent shows a tree it believes is fresh.

---

## Worked examples

All frames are snake_case, length-prefixed per §4.1. Headers are omitted for readability.

**Expanding `src/parser` — the common case.**

```json
{"jsonrpc":"2.0","id":"req_watch_017","method":"workspace/watch",
 "params":{"workspace_id":"ws_7f2a","paths":["src/parser"]}}
```

```json
{"jsonrpc":"2.0","id":"req_watch_017",
 "result":{"watching":12,"refused":[]}}
```

**Opening a file through search whose folder is not expanded (FR-003c).** The path is a file, and
the engine watches its parent directory without that directory entering the requested set.

```json
{"jsonrpc":"2.0","id":"req_watch_018","method":"workspace/watch",
 "params":{"workspace_id":"ws_7f2a","paths":["crates/codec/src/frame.rs"]}}
```

```json
{"jsonrpc":"2.0","id":"req_watch_018",
 "result":{"watching":13,"refused":[]}}
```

**A mixed call with three refusals.** The workspace is large, one folder was deleted on the host,
and one path is inside the exclusion set. The call succeeds; `watching` counts what was accepted.

```json
{"jsonrpc":"2.0","id":"req_watch_019","method":"workspace/watch",
 "params":{"workspace_id":"ws_7f2a",
           "paths":["src/lexer","node_modules/left-pad","docs/archive","vendor/rocksdb"]}}
```

```json
{"jsonrpc":"2.0","id":"req_watch_019",
 "result":{"watching":14,
           "refused":[{"path":"node_modules/left-pad","reason":"excluded"},
                      {"path":"docs/archive","reason":"not_found"},
                      {"path":"vendor/rocksdb","reason":"capacity"}]}}
```

`src/lexer` is watched. The developer is told that three folders will not report changes
(FR-005), the workspace stays open and browsable (FR-005a, FR-027, SC-009c), and nothing about
this response is an error.

**Collapsing `src/parser` while `src/parser/expr.rs` is still open (FR-004, FR-003c).**

```json
{"jsonrpc":"2.0","id":"req_watch_020","method":"workspace/unwatch",
 "params":{"workspace_id":"ws_7f2a","paths":["src/parser"]}}
```

```json
{"jsonrpc":"2.0","id":"req_watch_020","result":{"watching":13}}
```

`src/parser/expr.rs` was requested in its own right when the tab opened, so it is still in the
set, its parent directory is still watched, and the open tab is still reported on. Zero host
watches were released (SC-009b).

**Re-establishment after a drop (FR-026b, SC-012b).** One call, the whole set, folders and files
together.

```json
{"jsonrpc":"2.0","id":"req_watch_021","method":"workspace/watch",
 "params":{"workspace_id":"ws_7f2a",
           "paths":["src","src/parser","src/lexer","crates/codec/src/frame.rs",
                    "src/parser/expr.rs"]}}
```

```json
{"jsonrpc":"2.0","id":"req_watch_021",
 "result":{"watching":5,"refused":[]}}
```

`watching` is 5 and not 13: the engine restarted, its set was empty, and this call established
it. A client that read this number as "13 as before" would be reading a field that has no such
meaning.

**A path escaping the root — a client bug, refused as one.**

```json
{"jsonrpc":"2.0","id":"req_watch_022","method":"workspace/watch",
 "params":{"workspace_id":"ws_7f2a","paths":["src","../../etc"]}}
```

```json
{"jsonrpc":"2.0","id":"req_watch_022",
 "error":{"code":-32002,"message":"path refused: outside the workspace root"}}
```

`src` is **not** watched. The call changed nothing, which is what "fails the whole call" means,
and the refusal is identical whether or not the escaped target exists (F003's FR-007, §4.7).

---

## Amendments this feature made to the system specification

**All applied 2026-09-24**, before implementation, as Principle II requires. Recorded here so a
reader of this contract can confirm the catalogue describes it rather than assuming so.

1. **§4.8 gained two rows**, `workspace/watch` and `workspace/unwatch`, with the paragraph
   explaining why watching is client-driven and why `paths[]` carries what the client cares about
   rather than the directories the engine will watch.
2. **§10.3 was narrowed** from "scoped to the workspace" to scoped to what the client has asked
   for, with the reason and the kernel-overflow route.
3. **Appendix A gained A-WATCHSCOPE**, with A-COALESCE, A-UNPROVEN and A-WATCHLOCAL alongside it.
   Appendix A went from 31 records to 35.
4. **§6.1's `watch()` was corrected.** It declared
   `async fn watch(&self, path: &RelPath) -> Result<WatchHandle>` — one path, and a handle to a
   `WatchHandle` type that existed nowhere in the codebase — while the client-side port declared
   `watch(&self, ws, path) -> ProviderResult<()>` with the handle already dropped, and this
   contract needs a set of paths and a partial result. Three shapes, and §6.1 called its own
   normative. It now reads
   `async fn watch(&self, paths: &[RelPath]) -> Result<WatchOutcome>` with `unwatch` beside it and
   no handle. Phase 0 had listed only §4.8 as the Principle II blocker; this defect sat one section
   away and was found by writing this contract against the catalogue rather than against the plan.

## What is NOT added here

`workspace/setWatched` — rejected in research.md, *Protocol additions*: every expand and collapse
would resend the entire set.

A depth or recursion field on `paths[]` — deferred by the same section's reversal conditions. A
directory in the set means its immediate children, exactly as `workspace/readDirectory` means
immediate children (§10.1), and recursion is the cost FR-003 exists to avoid.

A new `-32000`-range error code for exhausted capacity. §4.4 has none and needs none: guarantee 8
makes exhaustion a refusal in a successful result, because FR-005a forbids it failing the call.
**This is stated rather than assumed, because a reader checking §4.4 for a watch-specific code
will not find one and should know that is deliberate.**
