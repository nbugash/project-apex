# Research: File Watch Sync

**Branch**: `feature/F004-file-watch-sync` | **Date**: 2026-09-24 | **Plan**: [plan.md](./plan.md)

Phase 0 output. Every unknown in the plan's Technical Context is resolved here, and every
decision that closes a genuine alternative names what it rejected and what would reverse it
(Principle III).

## Gate checks performed before research

**Principle IV — open items.** `grep -c 'OPEN: ' project-apex-predator.md` returns 19, and all
nineteen are resolution statements in Appendix A or prose in Appendix B. Appendix B states:
"All items resolved 2026-09-23. Nothing here blocks a feature." No live marker sits in §4.8,
§10.3, §10.4 or §5.2. **Gate passes.**

**Principle II — contradiction.** One found, blocking, resolved below under *Protocol
additions*.

---

## Protocol additions

**Decision**: Add two requests to §4.8 — `workspace/watch` and `workspace/unwatch`, both taking
a **list** of workspace-relative paths and both idempotent against a set the engine holds per
workspace.

```
workspace/watch    request   workspaceId, paths[]   ->  {watching, refused[]}
workspace/unwatch  request   workspaceId, paths[]   ->  {watching}
```

`refused[]` carries `{path, reason}` for paths the host could not watch, which is how FR-005
reports exhausted capacity without failing the call — FR-005a requires the workspace to stay
usable, so a partial result is the correct shape and an error is not.

**Rationale**: The first clarification scoped watching to what the developer has expanded and
opened. The engine cannot infer either, so the client must be able to say. §4.8 has
`workspace/onFileEvent` and `workspace/invalidateAll` as notifications and no request to begin
or end a watch, while §6.1 declares `watch()` on the provider trait — the catalogue and the
trait disagree, and Principle II makes that a contradiction to resolve in the system
specification before dependent work starts. This is the same absence F003 found with
`workspace/register`, and it is the fifth of its kind.

Taking a list rather than one path is what makes reconnection honest. FR-026b requires watches
to be re-established for everything still expanded and open; with set semantics that is one
call carrying the current set, and the engine reconciles. A per-path API would make the client
replay a remembered history, and a client that mis-remembers resumes believing it is being
told about changes when it is not — FR-025's failure arriving by another route.

**Alternatives considered**:

- *Watch the whole workspace from `workspace/register`, client filters.* Needs no new method
  and is what §10.3 reads like today. Rejected: it exhausts host watch capacity on a large
  repository, which is the failure the clarification was asked about, and it makes the client's
  filtering the only thing standing between a `node_modules` install and a flood.
- *A single `workspace/setWatched(paths[])` carrying the complete desired set every time.*
  Attractively stateless. Rejected because every expand and collapse would resend the entire
  set; the delta shape costs one extra method and keeps the common message small, while
  `watch()` with the full set remains available for reconnection precisely because the
  operation is idempotent.
- *Notification rather than request.* Cheaper on the wire. Rejected: `refused[]` has to come
  back, or FR-005 cannot be satisfied — a silent failure to watch is the exact thing the
  requirement forbids.

**Reversal conditions**: If watch establishment is measured outside §1.4's budget because a
round trip per expand is too slow, the call becomes fire-and-forget with refusals arriving as
a separate notification. If a future feature needs recursive watching of a subtree, `paths[]`
gains a depth field rather than a third method.

**System specification amendment required before implementation**: §4.8 gains both rows and the
paragraph explaining why watching is client-driven; §10.3's sentence "The engine owns all file
watches, using `inotify` scoped to the workspace" is narrowed to say scoped to what the client
has asked for, with the reason. Appendix A gains **A-WATCHSCOPE**.

---

## Watch scope: what is actually watched

**Decision**: The engine attaches watches to **directories**, never individual files. But the set
the client sends is not the set of directories — it is the set of **things the client cares
about**: folder paths for expanded folders, **file paths** for open tabs. The engine derives the
host watch set from it: the folders themselves, the parent directory of every named file, every
ancestor up to the workspace root, and the root itself, which is watched from registration and
released only when the workspace closes.

The distinction is load-bearing and was found during Phase 1 reconciliation. If the client sent
only directories, a folder holding an open tab would arrive as one path for two reasons, and
`unwatch` on a collapse would be indistinguishable from `unwatch` on a tab close — so collapsing a
folder would silently stop reporting a file still open inside it, which FR-003c and FR-004
explicitly forbid and SC-009b tests for. Sending reasons rather than conclusions keeps the
arithmetic on the side that has both facts, and makes the set self-describing: the engine can
always recompute the host set from it, which is what makes reconnection one idempotent call.

**Rationale**: `inotify` attaches to directories and reports events for their immediate
children; watching a file directly is possible but costs one watch per file and misses the
creation of siblings, which is most of what the tree needs. Ancestors are watched because a
rename of an ancestor changes every path beneath it and no descendant watch would see it. The
root is always watched because its disappearance is the `-32009` condition, and a workspace
that has lost its root must stop presenting a cached projection as a live view.

FR-003c is what makes the open-tab parents necessary: a file can be open without its folder
being expanded, reached through search or expanded-then-collapsed, and FR-023 promises that
file is reported.

**Alternatives considered**:

- *Recursive watch of each expanded folder.* `inotify` has no recursive mode; recursion means
  walking and adding a watch per descendant directory, which reintroduces the cost FR-003 exists
  to avoid the moment somebody expands a folder containing a large subtree.
- *Watch files individually for open tabs.* One watch per tab rather than per containing
  directory. Rejected as strictly worse: the same cost, and it misses the case where the file is
  replaced by an atomic rename, which is how most editors save.

**Reversal conditions**: If tab-parent directories prove to dominate the watch count in practice
— many tabs scattered across many directories — the open-tab half becomes a per-file watch after
all, measured rather than assumed.

---

## The coalescing window

**Decision**: **100 milliseconds**, per path, flushed on a trailing edge.

**Rationale**: Derived from bounds rather than chosen by taste.

*Upper bound.* SC-001 gives 2 seconds from the write landing to the change being reflected. The
chain is window + serialise + transit + client apply + render, and §18.1's mock daemon models a
250 ms round trip. At 100 ms the window is about a fifth of the transit it sits in front of and
about a twentieth of the budget it must fit inside — roughly 400 ms end to end, leaving the
measurement room to be a measurement rather than a squeeze.

*Lower bound.* The window has to actually collapse something. Editors save by writing a
temporary file and renaming it over the target, which is two to three inotify events per save;
compilers and loggers append repeatedly. Below about 50 ms a burst survives as a burst.

The requirement FR-012 states is the one worth restating: the count of events delivered is
bounded by **elapsed time** rather than by writes. At 100 ms per path that is at most ten events
per second per path, whatever the writer does — which is what makes SC-007 assertable.

**Alternatives considered**:

- *A leading-edge flush with a quiet period.* Reports the first change immediately, which reads
  better for a single save. Rejected because the last write in a burst is the one whose content
  matters, and a leading edge reports the first and then debounces away the state the developer
  would actually fetch.
- *An adaptive window widening under load.* Rejected on FR-012's own terms: "coalesce when there
  are a lot" is not implementable and not testable, and an adaptive window is that sentence with
  arithmetic attached.

**Reversal conditions**: If measured p99 reflection approaches 2 seconds, the window shrinks
before anything else is tuned. If ten events per second per path proves enough to delay
interactive traffic under §4.6, the window widens and SC-001's budget is re-derived.

---

## The bulk threshold

**Decision**: **256 distinct paths within a rolling 1-second window** becomes one
`workspace/invalidateAll` instead of individual events. The bulk window is deliberately not the
coalescing window: coalescing is per path over 100 ms, the bulk count is across paths over 1
second.

**Rationale**: Also derived from bounds.

*Lower bound — above any human action.* Editing touches single digits. A save-all in a large
project touches tens. A format-on-save across a directory touches tens. 256 sits an order of
magnitude above the largest of these, so normal work cannot trip it.

*Upper bound — the frame must not grow dangerous.* §4.1 caps a frame at 1 MiB and A-BULKSIZE
puts the bulk-transfer threshold at 512 KiB. A path event serialises to roughly 150–250 bytes,
so 256 of them is about 64 KiB — a factor of eight inside the smaller limit. A threshold in the
thousands would put a routine branch switch within reach of the frame cap, and §10.4 exists
precisely so that never happens.

**This argument only holds because events batch, which they did not when it was first written.**
§4.8's `workspace/onFileEvent` carries a singular `relativePath`, so 256 events would be 256
frames and no frame would approach any cap — the arithmetic above would be describing a message
shape the catalogue does not define. Phase 1 reconciliation caught it. The resolution is the next
section, and it makes the bound real rather than removing it.

What §10.4 gives is the cases — branch switches, large pulls — and no number. "Thousands" is not
a bound a test can assert against, which is why FR-015 required a stated count.

**Alternatives considered**:

- *A byte threshold rather than a count.* Closer to the real constraint, and rejected as harder
  to reason about: the developer-facing question is "did something wholesale happen", which is a
  count of paths, and a byte threshold makes the answer depend on path length.
- *Threshold on distinct top-level directories.* Would catch a branch switch more precisely.
  Rejected as more machinery for the same decision, and it misclassifies a large change confined
  to one directory as ordinary.

**Reversal conditions**: If a branch switch in a repository of realistic size produces fewer than
256 changed paths — so `invalidateAll` never fires in the case it exists for — the threshold
drops. If normal work trips it, it rises, and the first place to look is a tool writing into a
directory that should have been excluded.

---

## Event batching, and the metadata an event must carry

**Decision**: Amend `workspace/onFileEvent` twice. It carries an **array** of events in one
notification rather than one event per frame, and each event of kind `created` or `modified`
carries the entry metadata `readDirectory` already returns — `type`, `size`, `modified`.

**Rationale**: Two separate defects, found in Phase 1, with one amendment between them.

*Batching.* The coalescer already drains in batches: everything whose window closed at the same
instant comes out together. Writing those as one frame rather than as n frames is better for the
requirement that matters — FR-016 and §4.6 make this one pipe and one queue, and 256 separate
frames means taking the writer 256 times, interleaved with whatever interactive traffic is
queued behind them. One frame is one acquisition. The batched shape is also what makes the bulk
threshold's upper bound a real constraint rather than a description of nothing.

*Metadata.* A `created` event cannot produce a row in `files` without it. That table declares
`size_bytes`, `remote_modified_at` and `is_directory` all `NOT NULL`, and §4.8's event carries a
path and nothing else — so US1's first acceptance scenario, a new file appearing in an expanded
folder, is unsatisfiable as the catalogue stands. The alternatives were a follow-up `stat` per
created path, which FR-020 forbids and which puts a round trip inside the interaction budget, or
re-listing the parent, which is the same cost with worse granularity.

Carrying metadata does not breach FR-013. That requirement says an event carries no **content** —
"it says something changed, not what it now is". Size, type and modification time are what
`readDirectory` already returns for every entry; they are how the tree is drawn, not what the file
says. The distinction FR-013 protects is that no event may make the client believe it holds
current bytes, and metadata cannot do that: validity is still a hash comparison and nothing else.

**Alternatives considered**:

- *Leave the notification singular and drop the frame-cap argument.* Honest, and it leaves FR-016
  facing 256 writer acquisitions for one branch switch, which is the failure the threshold exists
  to prevent occurring inside the mechanism meant to prevent it.
- *Send a `stat` for each created path.* No wire change. Rejected on FR-020 and on the budget: a
  folder receiving twenty new files becomes twenty round trips before the tree is right.
- *Mark the parent stale on any creation and re-list lazily.* Cheapest, and it fails US1
  acceptance 1 — the file does not appear until the developer navigates away and back.

**Reversal conditions**: If batching is measured to delay interactive traffic more than the
per-frame writes it replaced — a single large frame occupying the queue where small ones could be
interleaved — the batch gains a size cap and splits, which is a change to the number of frames and
not to the contract.

---

## Kernel queue overflow

**Decision**: An `IN_Q_OVERFLOW` from the kernel is delivered to the client as
`workspace/invalidateAll` for that workspace.

**Rationale**: inotify's queue is finite. When a burst outruns the reader the kernel drops events
and says so once, and everything dropped is a change the client will never hear about. That is
exactly the silence FR-005 and FR-025 exist to forbid — a watcher that is running and telling
nobody.

Treating it as a wholesale invalidation is the same answer §10.4 already gives to the same
problem arriving by the expected route. The client's response is identical and already specified:
mark the tree stale, re-read lazily, discard no cached content. Nothing new has to be designed.

This condition is recorded nowhere in the specification, the feature spec or Phase 0 as first
written; it was found while specifying the watcher port. Because it closes an alternative it owes
an Appendix A record, which A-COALESCE absorbs rather than taking a fifth identity.

**Alternatives considered**:

- *Report it as "cannot watch" under FR-005.* Defensible, and wrong in effect: watching has not
  failed and does not need re-establishing, so the developer would be told they have lost
  freshness they still have.
- *Re-walk the watched directories and diff.* Recovers precisely which paths changed, and puts a
  filesystem traversal on the host at the exact moment the host is already overloaded.

**Reversal conditions**: If overflow proves common enough that wholesale invalidation is a
noticeable cost, the reader gets its own buffer sized against the burst, and overflow becomes the
rare case it should be.

---

## Rename detection

**Decision**: Pair `IN_MOVED_FROM` and `IN_MOVED_TO` by inotify's cookie within the 100 ms
coalescing window. A paired cookie is one `renamed` event naming both paths. An unpaired
`IN_MOVED_FROM` at flush time is a `deleted`; an unpaired `IN_MOVED_TO` is a `created`.

**Rationale**: FR-011 requires a rename to arrive as a single event naming both paths rather
than a deletion followed by an unrelated creation, because F003's cache survives a rename only
if the entry moves rather than being lost and re-found (its FR-022 and guarantee C9). inotify
already provides the pairing key; the only design question is how long to wait for the other
half, and reusing the coalescing window avoids a second timer with a second value to justify.

The unpaired cases are not a degradation, they are the correct classification: a file moved out
of the workspace genuinely is a deletion from this workspace's point of view, and one moved in
from outside genuinely is a creation.

**Alternatives considered**:

- *Infer renames by matching size and hash across a delete/create pair.* Works without cookies
  and is what a client would have to do if the engine did not pair. Rejected: it requires
  reading file content to classify a metadata event, which contradicts FR-013 — an event carries
  no content — and would make rename detection cost an I/O.

**Reversal conditions**: If pairing at 100 ms misses renames because the two halves are observed
further apart under load, the pairing window separates from the coalescing window and gets its
own value.

---

## Directory rename with a subtree

**Decision**: The engine emits **one** `renamed` event naming the directory. The client rewrites
every descendant path in its projection with a single prefix update, bounded to a separator
boundary.

**Rationale**: FR-022 requires a directory rename to be reflected for every descendant, none of
which was individually touched. Enumerating descendants engine-side would emit thousands of
events for one user action — the flood the bulk threshold exists to prevent, caused by the
feature meant to prevent it. The client already holds the subtree in `files`, so it can do the
rewrite locally with no additional round trip.

The separator boundary is the part that bites. Matching `relative_path LIKE 'src%'` also rewrites
`src-generated`, silently corrupting unrelated rows. The match must be the exact row
`relative_path = 'src'` together with `relative_path LIKE 'src/%'`, and that is a test before it
is an implementation.

Two consequences to carry into design. The rewrite fires F003's three FTS5 synchronisation
triggers once per affected row, so a large subtree rename is a bulk trigger run and needs
measuring rather than assuming. And the rewrite must be one transaction: a partially renamed
subtree is a projection that disagrees with itself, which is worse than a stale one.

**Alternatives considered**:

- *Invalidate the subtree instead of rewriting it.* Simpler, and it throws away the cached
  content under the renamed directory for a change that moved nothing — exactly what FR-021 and
  F003's C9 were written to prevent.

**Reversal conditions**: If the triggered FTS5 update on a large subtree is measured outside the
interaction budget, the rename marks the subtree stale and rewrites lazily, accepting a slower
first navigation in exchange for a bounded interaction.

---

## A watcher in a runtime-free engine

**Decision**: One dedicated OS thread owns the inotify file descriptor and the coalescer. It
polls the descriptor with a timeout equal to the time remaining in the nearest pending window,
drains whatever arrived, and writes due batches as notifications to stdout under the same mutex
the responder uses. No async runtime is added.

**Rationale**: `engine/Cargo.toml` records the constraint in its own comment — the engine "stays
synchronous and runtime-free, because it is embedded in the client and transferred on every
first connect". A watcher is not a reason to reverse a decision made about binary size and
transfer cost; `std::thread` plus a poll with a timeout does the whole job, and the stdlib rung
of the ladder is the right one to stop at.

Events are notifications, so they need no correlation with a pending request and the watcher
thread can serialise them itself. What it must not do is hold the writer for longer than one
frame, because §4.6 makes this one pipe and one queue and FR-016 forbids event delivery delaying
interactive traffic. That is a measurement obligation under Principle V, not a comment.

**There is no such writer today, and Phase 1 reconciliation found that this section originally
assumed one.** `engine/src/adapters/inbound/rpc.rs` has `Action` as `Reply | Nothing | Restart`;
`dispatch` returns frames and the session loop writes them, with nothing synchronising a second
writer because there has never been one. F004 is the first feature with an engine-originated
frame, so it introduces the seam: a writer owning stdout, taken briefly per frame, shared by the
session loop and the watch thread. `encode_notification` is also hard-typed to `&RestartNotice`
and must be generalised before it can carry a file event. Both are F004 tasks, and neither is a
detail — the seam is what FR-016's measurement measures.

**Alternatives considered**:

- *Add `tokio` and run the watcher as a task.* The ordinary answer, and it reverses a recorded
  decision to buy nothing this design needs — there is one descriptor and one timer.
- *Poll from the existing session loop.* No new thread, and it couples event latency to whatever
  the loop is doing, which is the coupling FR-016 exists to prevent.

**Reversal conditions**: If the stdout mutex is measured to delay interactive traffic, events
move to their own queue drained by the writer with interactive traffic given priority — which is
a change to §4.6's model and belongs in the system specification, not in this feature.

---

## Delivering a server-initiated frame in tests

**Decision**: Add one directive to the mock SSH daemon that emits a **caller-supplied opaque
frame** at a scripted moment. The mock never knows what it is sending.

**Rationale**: F004 is the first feature whose traffic includes a frame the engine originates.
Every directive the mock has — `echo`, `delay`, `drop`, `reorder`, `malformed`, `oversized`,
`stall`, `close-mid-frame` — is request-and-reply shaped, so there is currently no route by which
a notification can be delivered under latency or loss. Half of SC-001 and all of SC-005's delivery
behaviour depend on that route existing.

The mock's governing constraint must survive. `client/core/tests/mock_daemon/main.rs` contains
`the_mock_implements_no_engine_method`, which fails the build if any §4.8 method name appears
anywhere in that directory — the rule that stops the double acquiring engine behaviour one
reasonable-looking method at a time. A caller-supplied frame keeps it intact: the method name
lives in the test's own string, the mock carries only the framing, and the README's rule that a
directive stays "about framing, timing or the shape of the stream" is satisfied exactly. Emitting
an opaque frame at a chosen moment **is** a statement about the shape of the stream.

**Alternatives considered**:

- *Teach the mock to emit `workspace/onFileEvent`.* Direct, and it puts a §4.8 method name in the
  directory, failing the guard and beginning the drift the guard exists to prevent.
- *Test notification delivery against the real engine only.* No mock change, and it removes
  latency and loss from the one feature whose behaviour under latency and loss is the point.

**Reversal conditions**: If a later feature needs the mock to originate a *correlated* frame
rather than an opaque one, the directive is the wrong shape and the double needs re-founding.

---

## Where the exclusion set lives

**Decision**: The resolved exclusion set is computed **once per workspace at
`workspace/register`** and stored on the registered workspace, not on the watcher. Any consumer
— the watcher now, the indexer when it exists — reads it from there.

**Rationale**: A-IGNORE and FR-007 require one set shared by the indexer and the watcher, because
an indexer that indexes what the watcher ignores returns search results for files whose changes
are never noticed. Nothing in the engine computes such a set today and there is no indexer yet —
`workspace/search` is not implemented — so F004 is where the set is born, and the question is
how to stop a second one appearing later.

Storing it on the registered workspace makes the sharing structural rather than conventional. A
watcher-private set would satisfy F004 and leave the indexer free to compute its own, and
"somebody will remember" is exactly the assumption A-IGNORE was written to remove.

The content is fixed by §10.3: the repository's own `.gitignore` files plus `.git/`,
`node_modules/`, `target/`, `dist/`, `build/`, `.venv/`, `__pycache__/`. No per-workspace user
configuration in v1 (FR-009).

**Alternatives considered**:

- *Compute it lazily on first watch.* Defers work nobody has asked for, and makes the indexer's
  first call and the watcher's first call race to define it.
- *Recompute per watch call so `.gitignore` edits take effect.* Correct in a narrow sense and
  rejected on cost: the set is walked from disk, and re-walking on every expand puts a filesystem
  traversal inside the interaction budget. A `.gitignore` edit taking effect on re-registration
  is an acceptable staleness, and it is recorded here so it is a decision rather than a bug.

**Reversal conditions**: If `.gitignore` edits during a session prove to matter in practice, the
set is invalidated by an event on a `.gitignore` path — which the watcher is already positioned
to deliver.

---

## Representing unproven content

**Decision**: A boolean `unproven` column on `file_contents`, set when an event names a cached
file, cleared when a hash comparison runs. `Validity` is untouched.

**Rationale**: The spec is explicit that unproven "is not a third validity state — it is a hint
that the existing hash check will disagree". F003 built `Validity` with exactly one constructor
taking two hashes, deliberately, so that no path exists by which anything other than a hash
comparison can declare content valid. Adding a variant would reopen that. A separate flag keeps
the hash as the only thing that decides, which is FR-019's requirement stated as a type.

Tree staleness is the second column, on `files`, for the same reason it is a separate concept:
FR-017 and FR-026 mark tree regions stale, and the spec's key entities keep staleness and
content validity apart on purpose.

Together these take the projection from `user_version` 1 to 2. F003 built the migration
machinery; this is its first use, which makes the migration test as much the point as the
columns.

**Alternatives considered**:

- *A third `Validity` variant.* The obvious shape, and it puts a non-hash path into the one type
  built to have none.
- *Derive unproven from a timestamp comparison.* No schema change, and it makes "has this been
  disproved" a clock question, which is how a cache starts trusting clocks.

**Reversal conditions**: None foreseen. If a second hint of this kind appears, the two become an
explicit flags column rather than accumulating booleans.

---

## Local mode does not watch in this feature

**Decision**: `LocalWorkspaceProvider::watch()` returns `Unsupported`. F004 ships no native
watcher for macOS or Windows, and no inotify in the client crate for Linux local mode.

**Rationale**: The product is a thin client against a remote engine; the local provider exists
so the read path can be exercised without a host, which F003 established. Native watching would
mean FSEvents on macOS, `ReadDirectoryChangesW` on Windows and a second inotify integration on
Linux — three backends serving a mode no requirement asks to watch.

FR-027 already specifies the behaviour this produces: losing the ability to watch must not make
the workspace unusable, browsing and reading continue, and the loss is stated. So the degradation
path is not a gap, it is a requirement with a test, and local mode exercises it for free.

**Alternatives considered**:

- *Use the `notify` crate for a cross-platform watcher everywhere, client and engine.* One
  dependency, all platforms. Rejected for the engine on binary size, which A-BOOT makes a
  first-class concern because the engine is transferred on every first connect, and rejected for
  the client because nothing requires it.

**Reversal conditions**: A local-first mode with real users, or F005's cloud burst wanting a
local workspace watched, reverses this — and the port is already the seam that makes it a new
adapter rather than a change.

---

## Appendix A records required before implementation

Principle III requires each of these in the system specification before the code implementing it
is written. Four decisions close genuine alternatives:

| Record | Decision | Reversal condition |
|---|---|---|
| **A-WATCHSCOPE** | Watching is client-driven; the client names what it cares about — folders and open files — and the engine derives the host watch set; `workspace/watch` and `workspace/unwatch` added to §4.8 | Watch establishment measured outside §1.4, or a feature needing recursive subtree watching |
| **A-COALESCE** | 100 ms trailing-edge coalescing per path; 256 distinct paths within 1 second becomes `invalidateAll`; a kernel queue overflow is delivered the same way | Measured p99 reflection approaching 2 s; or `invalidateAll` never firing on a real branch switch |
| **A-UNPROVEN** | Unproven is a flag beside validity, never a validity state; `Validity` keeps its single hash-taking constructor | None foreseen |
| **A-WATCHLOCAL** | Local mode does not watch in v1; the degradation path FR-027 specifies covers it | A local-first mode with users, or F005 needing a watched local workspace |

The system specification needs six edits, not the two Phase 0 first listed. Phase 1 found the
other four by trying to write the contracts against what §4.8 and §6.1 actually say:

| Section | Edit | Why |
|---|---|---|
| §4.8 | Two method rows for `workspace/watch` and `workspace/unwatch` | A-WATCHSCOPE; the fifth absence of this kind |
| §4.8 | `workspace/onFileEvent` carries `events[]`, and each event carries `type`, `size`, `modified` | *Event batching, and the metadata an event must carry* |
| §4.8 | Enumerate the `event` vocabulary — `created`, `modified`, `deleted`, `renamed` | The catalogue names the parameter and never its values, so `renamed`, which FR-011 makes mandatory, is undiscoverable from the specification |
| §6.1 | `watch()` becomes `watch(&self, paths: &[RelPath]) -> Result<WatchOutcome>`; `unwatch` added; `WatchHandle` removed | §6.1 calls its signature normative and declares `watch(path) -> WatchHandle`. `WatchHandle` exists nowhere in the codebase, F003's stub already dropped it, and set semantics replace it. Three shapes, one of them normative — Principle II makes this blocking, and Phase 0 listed only §4.8 |
| §10.3 | Narrowed from "scoped to the workspace" to scoped to what the client asked for | A-WATCHSCOPE |
| §5.2 | `files_fts_update` narrowed to `AFTER UPDATE OF relative_path, name ON files` | Marking the tree stale is an UPDATE that changes no indexed term, and the trigger as written fires a delete-and-reinsert pair per row regardless. FR-017 and FR-026 mark the **whole** workspace stale, so that is 2N pointless FTS5 writes. Narrowing the trigger removes it entirely |

Not amended, deliberately: §4.4 gains no code for exhausted watch capacity, because capacity is
reported in `refused[]` rather than as an error. Recorded here so a later reader knows the absence
was decided rather than overlooked.

Two defects adjacent to this feature were found and are **not** F004's to fix, recorded so they
are not lost: `rpc.rs` maps every use-case error from `readDirectory`, `stat` and `readFile` onto
`-32003`, so a permission failure is reported to the developer as "no such file or directory"; and
`-32000` has no row in §4.4's table while `rpc.rs` writes it as a bare literal, in defiance of
`wire.rs`'s own rule that neither side may write the integer inline.
