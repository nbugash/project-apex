# Phase 0 Research: Workspace Cache

**Branch**: `feature/F003-workspace-cache` | **Date**: 2026-09-23 | **Plan**: [plan.md](./plan.md)

Every decision below closes an alternative. Three are marked **Promote** — they bind features
beyond this one, so Principle III wants them in Appendix A of the system specification rather
than only here.

No `NEEDS CLARIFICATION` markers entered this phase: the specification's five clarifications and
`§1.4`, `§4.x`, `§5.x`, `§6.x`, `§10.1` between them left no functional unknown. What follows is
entirely technical selection and shape.

---

## The SQLite driver

**Decision.** `rusqlite`, with the `bundled` and `fts5` features.

**Rationale.** Principle VIII names `rusqlite` explicitly as an example of a framework type that
must stay inside adapters, which is a decision already taken in everything but the manifest.

`bundled` matters more than it looks. The schema requires FTS5, and FTS5 is a *compile-time*
option in SQLite. A system `libsqlite3` without it turns `CREATE VIRTUAL TABLE ... USING fts5`
into a runtime error on a user's machine and nowhere else — the exact failure mode that is
invisible in CI and unfixable by the developer who hits it. Bundling makes the SQLite build a
property of our artifact rather than of the host.

**Alternatives considered.**

*`sqlx`.* Compile-time-checked queries are genuinely valuable and its async API would remove the
blocking-pool handoff below. Rejected on two counts: its SQLite backend drives the same
synchronous C library on a background thread, so the handoff is hidden rather than removed; and
compile-time verification requires either a live database at build time or a checked-in query
cache, which makes the build depend on a database file. For a projection with perhaps twenty
distinct statements, that is a large mechanism for a small problem.

*`libsqlite3-sys` directly.* Nothing to gain. We would reimplement `rusqlite`'s row mapping and
its safety invariants, and the unsafe surface is precisely what should not be bespoke here.

*A non-SQL embedded store (`redb`, `sled`).* Rejected outright: §5.2 is canonical and specifies
SQL, WAL and FTS5. Substituting a key-value store would contradict the source of truth, which is
what Principle II forbids.

**Reversal conditions.** FTS5 proving inadequate for path search at scale, which reopens §5.2
rather than this entry.

---

## How the async provider reaches a synchronous database

**Decision.** The SQLite adapter owns **one** connection behind a mutex, and every call crosses
into `tokio::task::spawn_blocking`.

**Rationale.** SQLite's C library is synchronous. Calling it directly from an async task blocks a
runtime worker thread for the duration, and the runtime cannot know it has been stalled — which is
how a fast query and a slow one become indistinguishable to everything else scheduled on that
thread.

One connection rather than a pool because of what actually contends. This is a single-user desktop
application; reads come from one interface at a time, and the only sustained writer is
maintenance, which by FR-026a runs before any workspace is open. A pool would add configuration,
a checkout path and a failure mode in exchange for parallelism that has no second party. WAL's
one-writer-many-readers property is the argument *for* a pool, and it only pays when there are
concurrent readers to serve.

The 1 ms p99 target (§1.4, SC-004c) is the thing this must not break, and it is why the decision is
made here rather than left to a profiler. A `spawn_blocking` handoff on a warm pool is a thread
wake — tens of microseconds — against an indexed single-table query. The measurement in SC-004c is
what settles it; this paragraph only says what is expected to be measured.

**Alternatives considered.**

*A dedicated cache thread with a command channel.* Strictly more control, and the shape to reach
for if contention ever appears. Rejected now because it requires enumerating every operation as a
message type, which is a hand-written layer over what `spawn_blocking` already provides.

*Blocking calls straight from async code.* Rejected. It works, until the day a large eviction or a
cold page fault stalls the runtime worker that was also going to render the sidebar.

*A connection pool (`r2d2_sqlite`).* Rejected for the contention argument above. The reversal
condition names exactly when to revisit.

**Reversal conditions.** A measured p99 regression attributable to connection contention — which
means concurrent readers exist, which means background prefetch (§11.4, a later feature) has
arrived. Revisit at that point rather than pre-empting it.

---

## Making `WorkspaceProvider` dynamically dispatchable

**Decision.** Add the `async-trait` crate and declare the port `#[async_trait]`, exactly as §6.1
writes it.

**Rationale.** FR-001 requires that consumers cannot tell a local provider from a remote one,
which means the concrete type is chosen at runtime and the trait must be `dyn`-compatible. Rust
1.75's native `async fn` in traits is not: it desugars to an anonymous associated future type,
which cannot be made into a trait object.

F001's `RequestTransport` uses the native form with `#[allow(async_fn_in_trait)]`, and that is
correct *there* — the transport is selected once, at composition, by a generic parameter, and never
varies at a call site. The difference is not stylistic. It is that one port has two
implementations chosen at runtime and the other does not, and the two ports should not be made to
match at the cost of the requirement.

§6.1 already writes `#[async_trait]`. Following the normative signature is the cheapest way to
stay out of Principle II's way.

**Alternatives considered.**

*Enum dispatch — one enum with a `Local` and a `Remote` variant.* No allocation, no macro, and
exhaustive. Rejected because it inverts the dependency the port exists to create: adding F015's
local provider would mean editing the enum in the application layer, so the application would know
every adapter by name. Ports and adapters is precisely the arrangement that prevents that.

*Generics threaded through the application layer.* Viable until a type must be stored in the Tauri
state alongside others, at which point the parameter propagates into every holder. Rejected for
the infection.

**Reversal conditions.** Rust making native AFIT `dyn`-compatible, at which point the macro is
deletable with no design change.

---

## Where cache policy lives

**Decision.** `CachedWorkspace` is a **use case in the application layer** that implements
`WorkspaceProvider` and holds four ports: an inner `WorkspaceProvider` (the engine),
`WorkspaceCache`, `Clock` and `ConnectionStatusSource`.

**Rationale.** Everything it does is a rule, not a mechanism: a hash match serves from the cache
(FR-019); a mismatch fetches and replaces (FR-019); nothing is served unverified while connected
(FR-021a); a confirmation that exceeds its limit ends and offers the cached copy marked unverified
(FR-021c); while disconnected, content is served and marked possibly stale (FR-032); every hit
records an access (FR-028); a failure to cache never fails the read (FR-034). Principle VIII puts
rules in the application layer and keeps them out of adapters.

The practical consequence is the point. With this shape, US2, US4 and US5 are exercised against an
in-memory fake cache, a fake engine and a fake clock — no SQLite file, no process, no sleeping to
age content past fourteen days. That is Principle VII's unit level being cheap, which Principle
VIII's rationale says is one of the reasons the principle exists.

**Alternatives considered.**

*A `CachingWorkspaceProvider` decorator in `adapters/outbound/`.* The conventional placement, and
the first thing that comes to mind. Rejected because it puts cache validity and staleness marking
in an adapter, where Principle VIII forbids business rules and where testing them would require
the real store.

*Cache lookups inside the SQLite adapter, with the provider asking it first.* Rejected: it spreads
one policy across two layers, so the answer to "when is cached content served?" stops having a
single location.

**Reversal conditions.** None foreseen. A change here would be a change to Principle VIII.

---

## Directory pagination — and why it does not bump the protocol version

**Decision.** `workspace/readDirectory` gains two optional parameters, `cursor?` and `limit?`, and
an optional result field `nextCursor?`. Entries are ordered by `(is_directory DESC, name ASC)` —
the same order §5.4's sidebar query uses — and the cursor is the last `name` emitted. Default and
maximum page size is 1000 entries (FR-024).

**This amends §4.8 of the system specification and MUST land there before the code.**

**Rationale.** FR-024 establishes the need and the number; this entry decides the shape. A
name-based cursor is stateless on the engine: resuming means "the first entries after this name in
this order", which needs no server-side iterator, survives a restart, and cannot leak a handle.
Offset-based paging would silently skip or repeat entries when a directory changes between pages;
name-based paging can still miss a concurrently created entry, but it can never duplicate or skip
one that was stable, which is the failure mode a developer would actually notice.

**The version consequence is the interesting part.** §4.8's own rule says that adding an optional
parameter or adding a field to a result does **not** increment `protocolVersion`; only removing,
renaming, retyping or making an optional parameter required does. So this change does not force a
redeployment to every host. F002 added that rule while reasoning about `session/onRestart` and
observed that without it, adding a notification would have forced exactly such a redeployment. It
pays for itself here, one feature later, for a change that was not anticipated when it was
written. Ordering is part of the contract precisely because the cursor depends on it; a later
change to the sort order *would* be breaking, and contracts/workspace-methods.md says so.

**Alternatives considered.**

*Offset and limit.* Simpler and wrong under concurrent modification, per above.

*An opaque server-side cursor token.* Allows richer resumption, costs server state with a lifetime
and an eviction policy of its own. Rejected as unnecessary for a sorted directory listing.

*Leave it unpaged and cap the directory size.* Rejected by FR-024's arithmetic: a hundred thousand
entries at roughly a hundred bytes each is an order of magnitude past §4.1's 1 MiB frame cap, so
the listing is not slow — it is undeliverable.

**Reversal conditions.** A directory representation with a stable per-entry identity, which would
allow a cursor that survives renames.

---

## The bulk threshold — when a read leaves the control channel **[Promote to Appendix A]**

**Decision.** A single read whose raw payload would exceed **512 KiB** is fetched beside the
channel, over its own `ssh` invocation on the existing control master, per A-BULK. Reads at or
under it travel as `workspace/readFile` frames. A whole-file read of a file larger than 512 KiB is
therefore a bulk fetch; a ranged read of its first screen is not.

**Rationale.** §4.1 caps a frame at 1 MiB and §4.6 gives the reason — one pipe is one queue, so a
large response serialises ahead of every interactive request behind it. The threshold must
therefore sit below the cap with room for what the encoding adds, not at it. `workspace/readFile`
returns `{content, encoding, ...}`, and binary content (FR-003 forbids assuming text) is base64,
which is 4 bytes out for every 3 in. 512 KiB raw becomes roughly 683 KiB encoded, leaving about
340 KiB of headroom inside the cap for the JSON envelope, the path and the hash. A threshold of
768 KiB raw would encode to 1 MiB exactly and fail on the first frame with a path in it.

This is also what reconciles FR-023 with FR-025, which read as if they conflict. FR-023 wants a
large file readable in ranges so the developer sees the beginning immediately; FR-025 wants
anything that would not fit in a message off the channel entirely. Both hold, because they govern
different requests: the *first screen* is a small ranged read and goes through the channel, where
it is fast and cannot block anything for long; the *whole file* is bulk and goes beside it. §4.6
rule 3 describes precisely this and is the reason the two requirements were never in conflict.

**Promote** because F017's artifacts and F013's search results face the same choice, and deciding
it once in Appendix A is cheaper than three features each picking a number.

**Alternatives considered.**

*Set the threshold at the 1 MiB cap.* Off by the base64 expansion, so it fails on exactly the files
it was meant to permit.

*Route every file read through bulk.* One code path instead of two, and it moves a second `ssh`
invocation onto the critical path of opening a small file. A-BULK measured that invocation as free
in *authentication* terms over an existing master; it is not free in process-spawn terms, which is
what a sub-250 ms open budget notices.

*Chunk large reads into 1 MiB frames through the channel.* Rejected by A-BULK already: head-of-line
blocking, and a reassembly protocol §4 does not define.

**Reversal conditions.** A transport that multiplexes independent streams, which removes
head-of-line blocking and with it the reason for the threshold.

---

## The cache eligibility cap **[Promote to Appendix A]**

**Decision.** Content larger than **8 MiB** is read but never written to `file_contents`. The
`files` row still exists, `is_cached` stays 0, and the file is listed and navigable like any other.

**Rationale.** The specification's edge case requires that opening a multi-gigabyte artifact "must
not attempt to cache it whole, and must not fail in a way that suggests the file is broken", and
names no number. One is needed, because "large" is not a testable bound — the same defect the spec
itself found four times in its own adjectives.

8 MiB is chosen against what the cache is *for*. §5.5 and §5.6 describe a projection of source
code; §11.4's prefetch names manifests and recently changed files. The largest source files in
real repositories — generated parsers, vendored bundles, lock files — sit in the low single-digit
megabytes. 8 MiB clears them with room, while excluding the artifacts that would dominate disk use
for content nobody reads twice. Compressed at the SC-010 ratio, one file at the cap occupies about
4 MiB, so the cap also bounds the worst case a single entry can cost.

The honest consequence, stated rather than buried: **a file above the cap is never available
offline.** FR-033 covers it — it is reported as unavailable rather than shown empty — but a
developer who expects a 20 MiB generated file to be readable on a plane will not find it there.
That is the trade the number makes.

**Promote** because F012's offline editing must know which files can be edited offline at all, and
that answer is this cap.

**Alternatives considered.**

*No cap; cache everything read.* Simplest, and it lets one `git clone` of a repository with large
binaries fill the disk with content the fourteen-day window will not reclaim for two weeks.

*A total-size budget with LRU eviction instead of a per-file cap.* A better policy in the
abstract, and it contradicts §5.5, which specifies time-based retention and no size cap, and
A-WORKSPACE, which declined quotas on the grounds that the developer's disk is theirs to manage.
Changing that is a change to the source of truth, not a plan decision.

*Derive the cap from available disk.* Makes behaviour depend on the machine, so the same action
caches on one laptop and not another, and no test can assert either.

**Reversal conditions.** Evidence that real workspaces routinely hold source files above the cap,
or the arrival of a size-based budget in §5.5, which would replace this mechanism rather than tune
it.

---

## The confirmation limit **[Promote to Appendix A]**

**Decision.** The hash confirmation that FR-021a requires uses an explicit **2 second** timeout,
not the transport's 30 second default. On expiry the wait ends, the developer is told the content
could not be verified, and the cached copy is offered marked unverified (FR-021c).

**Rationale.** `Request` carries `timeout: Option<Duration>` with the comment that a caller
knowing its own budget should state it, and this caller knows its own budget. The 30 second default
is sized for a request whose failure is an error; this one's failure is a fallback, and thirty
seconds of a window that cannot be dismissed is indistinguishable from the hang FR-021b exists to
prevent.

2 seconds is 8× the §1.4 uncached interaction budget of 250 ms. Tighter values were considered and
rejected on a specific ground: a limit near 250 ms would expire routinely on ordinary
transcontinental latency under load, so the "unverified" marker would appear during normal
operation and developers would learn to ignore it — which costs more than the wait it saved. The
marker has to mean something. 2 seconds is comfortably past any healthy round trip and far short of
anything a person would call a freeze.

**Promote** because it is the first instance of a general rule this system will need repeatedly:
a request on the interaction path states its own limit, and that limit is derived from the §1.4
budget rather than from the transport default.

**Alternatives considered.**

*The 30 second transport default.* Free, and it makes a wedged engine look like a broken
application for half a minute.

*250 ms, equal to the interaction budget.* Correct as an expectation, wrong as a deadline —
budgets are p99 targets and deadlines must tolerate the tail the budget excludes.

*No limit, with a cancel control.* Puts the work on the developer for a condition the system can
detect itself.

**Reversal conditions.** Measured confirmation round trips whose p99 approaches the limit, which
would mean the limit is being set by the network rather than by the interface.

---

## The FTS5 synchronisation gap in §5.2

**Decision.** Add the three standard external-content triggers — after `INSERT`, after `DELETE`,
after `UPDATE` on `files` — to the canonical schema.

**This amends §5.2 of the system specification and MUST land there before the code.**

**Rationale.** §5.2 declares `files_fts` with `content='files'` and `content_rowid='rowid'`, which
makes it an *external-content* FTS5 table: it stores only the index and reads the column values
back from `files`. SQLite does not keep such a table in step automatically. As the schema is
written today, `files_fts` is created empty and stays empty, so every offline path search returns
nothing — and returns it quickly, with no error, which is the worst available failure. Nothing in
the schema or its three annotated corrections mentions this.

The triggers are the documented mechanism and make correctness structural: no write path can
forget to update the index, because no write path is involved. The alternative — having the
repository adapter maintain the index alongside each write — is one more thing every future write
must remember, and F004's watcher and F012's offline writes are exactly the future writes that
would forget.

**Alternatives considered.**

*Maintain the index from the adapter.* Above.

*A contentless or ordinary FTS5 table holding its own copy of the paths.* Removes the trigger
requirement by duplicating every path, which costs disk on the largest table in the schema and
introduces a second place a path can be wrong.

*Rebuild the index at startup.* Proportional to workspace size on every launch, which is the cost
§10's whole approach exists to avoid.

**Reversal conditions.** None. This is a defect in the schema rather than a choice between shapes.

---

## What the first schema version contains

**Decision.** Schema version 1 is the **whole** of §5.2 — including `git_status`, which no code in
this feature writes — plus the FTS triggers above. `PRAGMA user_version` carries the number.

**Rationale.** §5.2 is canonical under A-B5, so these tables are specified rather than anticipated;
this is not the speculative generality that should be deleted on sight. Creating the table now
costs one `CREATE TABLE` in the initial DDL. Creating it in F011 instead costs a migration, a
migration test, a second schema version, and a migration run on every installation in existence —
to add a table that was already written down.

`PRAGMA user_version` rather than a `schema_version` table because SQLite provides it in the file
header, it is read without a query, and it cannot itself be the thing that needs migrating.

What is deliberately **not** in version 1: A-OFFLINE's persisted outbox with base revisions. That
decision says it "becomes part of the cache schema (§5.2)", but §5.2 does not yet describe it and
F012 owns it. Adding a table this feature cannot specify would be inventing schema, which is the
opposite of the argument above.

**Alternatives considered.**

*Create only what F003 uses.* Defensible on YAGNI grounds and rejected because "specified in a
canonical schema" is the precise case YAGNI does not cover.

*A `schema_version` table.* Requires a query to read, and needs a migration path of its own for
the release in which it is introduced.

**Reversal conditions.** §5.2 growing tables whose shape is genuinely undecided, which would make
"create the whole schema" mean "invent the undecided parts".

---

## Migration atomicity, and why "half-transformed" is impossible

**Decision.** Each schema step runs inside a single SQLite transaction that also sets the new
`user_version`. A step that fails rolls back. A migration that cannot complete deletes the database
file and its `-wal` and `-shm` companions and rebuilds from empty, telling the developer that
cached content was rebuilt (FR-018b).

**Rationale.** FR-018c forbids reading a projection with a schema it was not written for "at any
point, including during a migration", and FR-018b forbids a readable half-transformed projection
"under any circumstance". Those are strong words and they are satisfiable structurally rather than
by care: SQLite's DDL is transactional, so a crash, a kill or a full disk mid-step leaves the file
exactly as it was, at the old version, and the next launch retries the same step. There is no
partial state to observe because there is no moment at which one exists.

That leaves one real failure — a step that runs to completion and produces something wrong, or
one that fails deterministically every launch. Neither is recoverable by retrying, and both are
why the discard path exists. Discarding is safe because the cache is a projection (FR-013, §5.1):
nothing is lost that cannot be fetched again. Refusing to launch, by contrast, would strand a
developer behind a cache the specification itself calls disposable.

The `-wal` and `-shm` files are named explicitly because deleting only the main database leaves a
WAL that SQLite will happily replay into the fresh one.

**Alternatives considered.**

*Copy to a new file, migrate the copy, swap on success.* Stronger against a corrupt original and
needs twice the disk for a cache whose worst case is measured in gigabytes. Rejected on that cost,
given transactional DDL already provides the atomicity.

*Refuse to launch and ask the developer.* Asks a person to decide about a disposable projection.

*Migrate lazily, per table, on first use.* Directly violates FR-018c.

**Reversal conditions.** A migration step SQLite cannot perform transactionally, which would make
the copy-and-swap approach necessary for that step.

---

## Hashing, and the second implementation that already exists

**Decision.** `sha2` (RustCrypto) in both the client and the engine.

**Rationale.** FR-019 makes a hash comparison the sole validity rule, and §5.6 requires it over
decompressed content so it compares directly with the engine's. It therefore runs on the
interaction path for every file open, over content up to the eligibility cap. `sha2` has
hand-written assembly and SHA-NI paths; a straightforward Rust implementation is several times
slower on exactly that workload.

There is already a second implementation in this repository: `client/core/build.rs` hand-rolls
SHA-256 to avoid a *build* dependency, because a build script cannot depend on a crate the build is
producing. That constraint is real and does not apply at runtime, so the two coexist for a stated
reason rather than by accident. F002 already tests that implementation against the system
`sha256sum` on a real 4.5 MB binary; this feature adds a test that the build-script digest and
`sha2` agree on the same bytes, so the two cannot drift apart silently.

**Alternatives considered.**

*Reuse the hand-rolled implementation at runtime.* No new dependency, and it puts a bespoke
cryptographic primitive on the hot path of every file open. The cost is paid per read, forever, to
avoid one well-audited crate.

*`ring` or `openssl`.* Both drag in far more than a digest; `openssl` adds a system dependency to a
binary that must deploy onto an arbitrary host.

**Reversal conditions.** None foreseen.

---

## The engine stays synchronous

**Decision.** No async runtime in the engine. The workspace use cases are synchronous, the
filesystem port is synchronous, and the stdio loop stays as F002 built it.

**Rationale.** Three reasons, in order of weight.

The engine is **embedded in the client binary and transferred over the wire on first connect**
(F002, A-BOOT), against a 30 second budget on a 10 Mbit/s link. Every megabyte of runtime is paid
on every first connect to every host. Tokio with the features this would need is not free at that
scale.

The engine has **nothing to overlap**. It reads one frame, answers it, reads the next. Its
concurrency comes from the fact that expensive work is not supposed to be on this channel at all —
which is what §4.6 says and what the bulk threshold enforces. An async runtime buys the ability to
interleave, and there is nothing to interleave with.

The blocking filesystem call is **the work**, not a wait. `read_dir` and `read` on a local disk are
CPU and page cache, not network latency. Async would move them to a blocking pool and arrive back
where it started, one runtime heavier.

The honest consequence: a single large read blocks the engine's loop for its duration. That is
bounded by the bulk threshold at 512 KiB, which is a few milliseconds of local disk, and reads
above it do not use this channel at all. The threshold is therefore doing double duty — it protects
the client's queue and it bounds the engine's loop — which is worth knowing before anyone
considers raising it.

**Alternatives considered.**

*Add tokio and make the engine async throughout.* Consistent with the client, and it buys nothing
the engine needs while costing deployment size on every connect.

*Spawn a thread per request.* Would let a slow read proceed while others are answered, and reorders
replies — which the correlation registry tolerates — at the cost of concurrent access to the roots
registry and an unbounded thread count under load. Revisit only if a measured stall appears.

**Reversal conditions.** A workspace method that genuinely waits — a watch, a subscription, a
long-poll. F004's file watching is the first candidate, and it is the right feature to reopen this
in, with a concrete need in hand.

---

## Path containment, and not leaking existence

**Decision.** The engine canonicalises the workspace root once at registration. For each request
it resolves the relative path **lexically** first — rejecting any component that is `..` or
absolute — joins it to the canonical root, then canonicalises the result and asserts the descendant
relationship again. Both assertions failing produce `-32002`, and so does a path that lexically
passes but resolves outside the root through a symlink.

**Rationale.** §4.7 and Principle VI require canonicalisation and a descendant assertion, and
FR-006 requires symlinks resolving outside the root to be refused. `std::fs::canonicalize` resolves
symlinks, which is what makes the second assertion catch them.

The two-stage shape exists because of FR-007. `canonicalize` fails on a path that does not exist,
so a single-stage check would answer "no such file" for an escape to a non-existent target and
"refused" for an escape to a real one — which tells an attacker whether `/etc/shadow` exists on the
host. The lexical stage rejects the escape before the filesystem is consulted, so every escape
produces the same `-32002` regardless of what is out there. A path that stays inside the root and
simply does not exist gets an ordinary not-found, which reveals nothing the caller was not already
entitled to know.

The client performs the same lexical validation when constructing a `RelPath`. FR-008 requires that
the engine's safety not depend on it, and the design honours that by the engine never receiving a
validated type — it receives a string off the wire and validates it itself, which is the only
arrangement in which "independently" is true rather than asserted.

**Alternatives considered.**

*Canonicalise only.* Fails FR-007 as above, and fails outright for a `stat` on a path that does not
exist, which is a legitimate request.

*`std::path::absolute` or manual normalisation without touching the filesystem.* Catches `..` and
misses symlinks entirely, which is the attack that matters when the engine holds the developer's
full filesystem rights.

*`openat2` with `RESOLVE_BENEATH`.* The strongest mechanism available and kernel-specific, Linux
5.6+. The engine also targets ARM64 instances and should not carry a syscall path with a fallback
that must be tested separately. Worth revisiting if the threat model ever includes a hostile host,
which A-EC2's single-tenant instance currently excludes.

**Reversal conditions.** A threat model in which the remote host is untrusted, which A-EC2 and
F002's threat model presently exclude and which would make TOCTOU between the two assertions worth
closing with `openat2`.

---

## Summary of amendments this plan owes the system specification

| Amendment | Where | Why it is owed |
|---|---|---|
| **`workspace/register` is added** — the catalogue defines no way to tell the engine what a `workspaceId` means, yet §15.4 step 3 and §4.4's `-32001` both presume it | §4.8 | Found during Phase 1 reconciliation. Without it, every other workspace method is unusable. No `protocolVersion` bump — adding a method is exempt. |
| `workspace/readDirectory` gains `cursor?`, `limit?`, `nextCursor?`, with the entry ordering stated as contractual | §4.8 | FR-024. No `protocolVersion` bump — §4.8's own rule exempts optional additions. |
| `files_fts` gains three external-content synchronisation triggers | §5.2 | Without them the table stays empty and offline path search silently returns nothing. |
| The bulk-read threshold | Appendix A | Binds F013 and F017. |
| The cache eligibility cap | Appendix A | Binds F012's answer to "what can be edited offline". |
| The confirmation limit, and the rule that interaction-path requests state a limit derived from §1.4 | Appendix A | Binds every later feature that waits on the engine while a developer watches. |
