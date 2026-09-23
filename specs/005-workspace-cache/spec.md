# Feature Specification: Workspace Cache

**Feature Branch**: `feature/F003-workspace-cache`

**Created**: 2026-09-23

**Status**: Draft

**Input**: Feature map entry F003 `workspace-cache` — the `WorkspaceProvider` trait with local
and remote implementations, the canonical SQLite schema with WAL, FTS and migrations, workspace
registration and path mapping, lazy shallow directory fetch, ranged file read with hash-based
cache validity, and Zstd content blobs with access-time eviction.

**Terminology**: the *engine* is the remote process F002 deploys (`ide-engine` in §4.8). The
*cache* is the client's local projection of a workspace. The *provider* is the interface every
consumer of workspace content uses, whichever side answers it.

## On the source of values

Every normative value here comes from the system specification or a recorded decision. Where
this document states a number it is quoting one.

- **§5.1** — the cache is a projection, never an authority. Where it disagrees with the engine,
  the engine wins.
- **§5.2 / A-B5** — the canonical schema, including the three corrections the review required:
  an opaque `file_id` so a rename **the projection is told about** preserves cached content,
  `last_accessed_at` so eviction is expressible at all, and an FTS table so offline path search does
  not degrade to a full scan. The qualifier is load-bearing and is why FR-022a exists.
- **§5.3** — a blob is valid when its hash equals the engine's. **Nothing else invalidates it**,
  and git status explicitly does not.
- **§5.5, §5.6** — fourteen days unopened, Zstd level 3, hash computed over decompressed content.
- **§6.1, §6.2** — the provider trait, and that a provider must not join untrusted input to a
  base path without asserting containment.
- **§10.1** — opening a workspace fetches only the root listing.
- **§1.4 / A-NFR** — the latency targets this feature owns, and how they are measured: p99, at the
  boundary between the interface and the transport, over at least 100 samples, with harness delay
  excluded and the measured value printed rather than only compared.
- **§4.1** — the 1 MiB frame cap, which is why a directory listing must be paged and why content
  above a derived threshold travels beside the channel.
- **§4.4** — the application error codes. This feature adds one, `-32009`, for a workspace whose
  root no longer exists; the plan records why it is not `-32001`.
- **§4.8** — the method catalogue. This feature implements its read methods and amends it twice:
  `workspace/register`, which the catalogue presumed in two places and defined nowhere, and
  pagination on `workspace/readDirectory`.
- **A-BULK** — bulk data travels beside the protocol channel, not through it.
- **A-WORKSPACE** — how a `workspaceId` is minted, that display names may collide, and that
  deletion removes both the remote directory and the local cache.
- **A-TEST / A-E2E** — the test levels every feature carries, and that end-to-end runs on Linux.
- **A-EC2** — the instance is single-tenant and the developer's own, which is why no disk quota is
  imposed on top of the retention window.
- **A-OFFLINE** — offline editing, which supersedes the read-only lock and belongs to F012. This
  feature stays read-only and does not implement it.
- **Constitution Principle VI** — the engine validates paths independently of the client. F002's
  plan recorded this obligation as landing here, because F002 had no workspace method to apply
  it to.

## What this feature is not

**It is not the editor.** Nothing here opens a buffer, renders text or saves. F006 does that,
and it consumes this feature's provider.

**It does not write.** The trait in §6.1 declares `write_file`, `create_file`,
`create_directory`, `rename` and `delete`, and this feature implements **none** of them. It
delivers the read path — `read_directory`, `stat`, `read_file` — end to end on both sides.

One word does double duty and is worth separating now. `rename` names two different things: the
**provider** method above, which nothing here implements, and an operation on the **local
projection**, which moves a cached file's path while keeping its content and which this feature does
implement and test (FR-022). The second exists so the first has something to call when F006 arrives.
Nothing in this feature invokes it, because nothing here performs a rename.

This is a scope boundary rather than an omission, and it is deliberate: writing brings
`baseSha256` conflict handling, which is F006's subject, and a cache that can be written to has
invalidation questions a read-only projection does not. The trait is declared whole because
§6.1 is normative; the methods this feature does not implement refuse rather than pretend.

**It does not watch.** `watch` is declared for the same reason and implemented by F004, which
owns file events and the client-side invalidation they drive.

**It does not recognise a rename it did not perform.** The projection can move a file it is told
about, keeping its content, and that operation ships here. But this feature learns of remote changes
only by re-listing a folder, which shows a name gone and a name added with nothing linking them. So
a rename made elsewhere costs a refetch. FR-022a records the limit; F006's write path is the first
caller with an identity to pass.

**It does not search content.** Offline *path* search is in scope, because the FTS table exists
for it. Content search runs on the engine and belongs to F013.

**It does not make the cache writable offline.** F012 does that, and A-OFFLINE governs it.

## Clarifications

### Session 2026-09-23

- Q: Does this feature implement the whole `WorkspaceProvider` trait? → A: No. It implements the
  read path on both sides; write, rename, delete and watch are declared and refuse, landing with
  F006 and F004. Recorded in "What this feature is not".
- Q: When a cached file is opened, does the editor wait for the engine to confirm the content is
  current, or show it immediately and reconcile? → A: Wait for confirmation — a stale byte is
  never shown while connected — **and** the interface must show that verification is in progress,
  so the wait is visible rather than a frozen window. Consequence: while online the cache saves
  transfer, not open latency.
- Q: What happens to a cache written by an older version of the application? → A: Migrate it in
  place, preserving cached content across upgrades, **and** show the developer that a migration
  is running. A migration that fails discards the projection and rebuilds rather than leaving a
  half-transformed one readable — that failure path was not part of the question and is chosen
  here, because refusing to launch over a cache the specification calls reproducible is not a
  defensible outcome.
- Q: When does eviction run? → A: Once at startup, before any workspace opens, so deleting can
  never compete with a read. Known limit accepted: a session that never restarts never evicts.
- Q: The engine implements no workspace method today. Does this feature add them? → A: Yes, on
  both ends — the same shape as F002, which had to build the engine before it could hand shake
  with one. The read methods of §4.8 are implemented here, with the path containment Principle VI
  requires.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Open a repository too large to fetch (Priority: P1)

A developer opens a workspace containing a repository with hundreds of thousands of files. The
tree appears immediately. Expanding a folder shows its contents without a perceptible wait, and
nothing the developer has not looked at is ever transferred.

**Why this priority**: This is the feature. Every technique in §10 follows from the client never
indexing, and a client that fetched a whole tree would be unusable on precisely the repositories
this product exists for. Until this works, there is nothing to look at.

**Independent Test**: Open a workspace with a deep, wide tree and confirm that the number of
directory listings requested equals the number of folders actually expanded, not the number that
exist.

**Acceptance Scenarios**:

1. **Given** a workspace that has never been opened, **When** the developer opens it, **Then**
   only the root listing is fetched.
2. **Given** a folder the developer has not expanded, **When** the workspace has been open for
   any length of time, **Then** its contents have not been fetched.
3. **Given** a folder expanded once, **When** it is collapsed and expanded again, **Then** no
   further listing is requested.
4. **Given** a repository of a hundred thousand files, **When** the developer expands ten
   folders, **Then** the work done is proportional to those ten folders rather than to the
   repository.

---

### User Story 2 - Reopen a file without waiting for the network (Priority: P1)

A developer reopens a file they were reading earlier. It appears immediately, from the local
projection, and the developer is never shown content that the engine would disagree with.

**Why this priority**: The cache exists to keep the interface off the network for content that
has not changed. Without validity that can be trusted, the only safe cache is no cache.

**Independent Test**: Open a file, change it on the remote side, reopen it, and confirm the new
content is served — then reopen an unchanged file and confirm nothing was transferred.

**Acceptance Scenarios**:

1. **Given** a cached file whose hash matches the engine's, **When** it is opened, **Then** it is
   served from the cache and no content is transferred.
2. **Given** a cached file whose hash no longer matches, **When** it is opened, **Then** the
   current content is fetched and the cache is updated.
3. **Given** any cached file being opened while connected, **When** its hash is being confirmed,
   **Then** the developer can see that verification is in progress rather than facing a window
   that appears frozen.
4. **Given** an engine that does not answer the confirmation within its limit, **When** the limit
   elapses, **Then** the developer is told the content could not be verified and is offered the
   cached copy marked as unverified — never an indefinite wait.
5. **Given** a file the developer has just edited and saved, making it modified in git, **When**
   it is opened, **Then** it is still served from the cache. Git status is not a cache signal,
   and treating it as one would discard exactly the files being worked on.
6. **Given** a file larger than a single message may carry, **When** it is opened, **Then** it
   arrives in ranges and the developer sees the beginning without waiting for the end.
7. **Given** a cached file that the projection is told has moved, **When** its path is updated,
   **Then** its content is still served from the cache and nothing is transferred.
8. **Given** a cached file renamed on the engine and noticed only when its folder is re-listed,
   **When** it is opened under its new name, **Then** it is fetched again and remains listed
   throughout — the content is lost, the file is not.

---

### User Story 3 - Address a workspace unambiguously, and know when it is gone (Priority: P1)

A developer opens two checkouts of the same repository. Each is a distinct workspace, neither
shadows the other, and closing one does not disturb the other's cache.

**Why this priority**: `workspaceId` is mandatory on every workspace method in §4.8, and a cache
keyed by anything else collapses two checkouts into one — which is the ordinary case for anyone
reviewing a branch beside their own work, not an edge case.

**Independent Test**: Register two workspaces whose display names collide, populate both, and
confirm each reads back its own content. Then remove one root on the engine and confirm the client
says so rather than continuing to browse it.

**Acceptance Scenarios**:

1. **Given** two workspaces with the same display name, **When** both are open, **Then** each
   resolves to its own content and neither shadows the other.
2. **Given** a workspace opened a second time, **When** it is registered, **Then** it attaches to
   the existing cache rather than building a second one.
3. **Given** a workspace being deleted, **When** the developer confirms, **Then** its cached
   content and its tree are both removed.
4. **Given** a workspace whose root has been deleted on the engine while it is open, **When** the
   developer next acts on it, **Then** they are told the workspace is gone rather than being shown
   its projection as though it were current.

---

### User Story 4 - Keep the cache healthy across time (Priority: P2)

A developer uses the same machine for months across many workspaces and several releases of the
application. Disk use stays bounded, the tree of a workspace they have not opened recently is
still navigable, and upgrading does not cost them their cached content or leave them staring at
a window that appears to have hung.

**Why this priority**: P2 because a cache that never evicts still works, right up until it does
not. This is what makes the product tolerable over months rather than days.

**Independent Test**: Age content past the retention window, run eviction, and confirm blobs are
gone while the tree is intact and files are marked uncached.

**Acceptance Scenarios**:

1. **Given** content not opened for longer than the retention window, **When** eviction runs,
   **Then** the blob is removed and the file is marked uncached.
2. **Given** evicted content, **When** the developer browses the tree, **Then** the structure is
   unchanged and the file is still listed.
3. **Given** an evicted file, **When** it is opened, **Then** it is fetched again and re-cached
   without the developer being told anything unusual happened.
4. **Given** any cache hit, **When** it is served, **Then** that file's last access is recorded,
   so the retention window measures use rather than age.
5. **Given** a running session, **When** the developer is working, **Then** no eviction runs —
   eviction happens at startup, before any workspace opens, so it never competes with a read.
6. **Given** a cache written by an earlier release, **When** the application starts, **Then** it
   is migrated with its content preserved and the developer can see the migration running.
7. **Given** a migration that fails partway, **When** the application recovers, **Then** the
   projection is rebuilt from scratch and the developer is told, and no half-transformed
   projection is ever read.

---

### User Story 5 - Find a file by path while disconnected (Priority: P2)

A developer loses their connection and can still find and read files they have already visited.

**Why this priority**: P2 because it depends on US1 and US2 having populated anything at all.
It is what makes an outage an inconvenience rather than a stop.

**Independent Test**: Populate a cache, disconnect, search by path fragment, and confirm results
come from the projection with no request attempted.

**Acceptance Scenarios**:

1. **Given** a disconnected client, **When** the developer searches for a path fragment, **Then**
   matches from the projection are returned without a request being attempted.
2. **Given** a disconnected client, **When** the developer opens a cached file, **Then** it is
   served, and the interface says the content may be stale rather than presenting it as current.
3. **Given** a disconnected client, **When** the developer opens a file that was never cached,
   **Then** they are told it is unavailable offline rather than shown an empty document.

---

### Edge Cases

- **A path that escapes the workspace.** A request for `../../etc/passwd`, or a symlink
  resolving outside the root. The engine must refuse it independently of anything the client
  checked, because the engine runs with the developer's full filesystem rights.
- **A confirmation that never returns.** The engine is reachable but wedged. The developer must
  not wait indefinitely for permission to read a file they already have.
- **A file that changes between the hash check and the read.** The content served must be
  internally consistent — never a hash from one version and bytes from another.
- **A file larger than the cache's practical limit.** Opening a multi-gigabyte artifact must not
  attempt to cache it whole, and must not fail in a way that suggests the file is broken.
- **A directory with an enormous number of entries.** Listing a folder with a hundred thousand
  children must not stall the interface or arrive as one unbounded message.
- **Binary content.** Images, PDFs and build artifacts are legal file content. Nothing may assume
  text, and nothing may corrupt bytes by guessing an encoding.
- **A rename.** A rename the client performed must keep the cached content, because the content is
  unchanged and refetching it would be wasted work. A rename merely *observed* when a folder is
  re-listed cannot be recognised as one — only a vanished name and a new one — so its content is
  dropped and refetched, and the file stays listed either way (FR-022a, FR-022b).
- **Two workspaces pointing at the same directory.** Legal, and each keeps its own projection.
- **The cache schema changing between releases.** An older cache migrates in place, visibly.
- **A migration interrupted halfway** — the machine sleeps, the process is killed, the disk
  fills. The next launch must not find a projection that is half one schema and half another and
  read it as either.
- **Disk full while caching.** Caching is an optimisation; failing to cache must not fail the
  read that prompted it.
- **Eviction meeting a workspace that is already open.** It must not run at all in that
  situation rather than running carefully.
- **A workspace deleted on the remote side while open.** The developer must learn the workspace
  is gone rather than browsing a projection of something that no longer exists.

## Requirements *(mandatory)*

### Functional Requirements

**The provider**

- **FR-001**: Every consumer of workspace content MUST reach it through one provider interface,
  which MUST NOT reveal whether the content is local or remote.
- **FR-002**: The provider MUST offer directory listing, metadata and ranged file reading.
- **FR-003**: File content MUST be carried as bytes, never as text. Binary artifacts are legal
  content, and an encoding guess corrupts them silently.
- **FR-004**: Methods this feature does not implement MUST refuse explicitly rather than appear
  to succeed.

**Path safety**

- **FR-005**: The engine MUST canonicalise every path it receives and assert it is a descendant
  of the workspace root, independently of any check the client performed.
- **FR-006**: A symbolic link resolving outside the workspace root MUST be refused.
- **FR-007**: A refused path MUST NOT reveal whether the target exists.
- **FR-008**: The client MUST also validate paths, and MUST NOT rely on that validation for
  safety. A client-side check protects against bugs, never against a stale or hostile client.

**Registration**

- **FR-009**: Each workspace MUST have an identity that is stable across sessions and
  independent of its display name.
- **FR-010**: Two workspaces MUST be distinguishable even when their display names are
  identical.
- **FR-011**: Re-opening a workspace MUST attach to its existing projection rather than create a
  second one.
- **FR-012**: Deleting a workspace MUST remove both its cached content and its tree.

**The projection**

- **FR-013**: The cache MUST be a projection and never an authority. Where it disagrees with the
  engine, the engine's answer MUST win.
- **FR-014**: Opening a workspace MUST fetch only the root listing.
- **FR-015**: Expanding a folder MUST consult the projection first, and MUST request at most one
  listing for that folder's immediate children on a miss.
- **FR-016**: Content already listed MUST NOT be re-requested while it remains valid.
- **FR-017**: The projection MUST survive a restart of the application.
- **FR-018**: A projection written by an older schema MUST be migrated in place, preserving its
  cached content across an upgrade.
- **FR-018a**: A migration MUST publish a state that says it is running, from before it begins
  until after it completes, and MUST refresh that state at least once per second while it runs.
  "Visible" alone is not a testable bound — one message at the start satisfies it and still
  leaves an upgrade that appears to hang, which is indistinguishable from a broken install.
- **FR-018b**: A migration that fails MUST discard the projection and rebuild it, and MUST tell
  the developer that cached content was rebuilt. A half-transformed projection MUST NOT be
  readable under any circumstance — it is the one state that could serve wrong bytes while
  believing they are right.
- **FR-018c**: A projection MUST NOT be read with a schema it was not written for, at any point,
  including during a migration.

**Validity**

- **FR-019**: Cached content is valid exactly when its hash matches the engine's for that path.
  **Nothing else** may invalidate it.
- **FR-020**: Git status MUST NOT invalidate the cache. Invalidating on it would discard cached
  content for exactly the files being worked on.
- **FR-021**: Content served MUST be internally consistent: the hash and the bytes MUST describe
  the same version.
- **FR-021a**: While connected, cached content MUST NOT be presented to the developer until the
  engine has confirmed its hash is current. A stale byte is never shown on a working connection.
- **FR-021b**: While a confirmation is outstanding, a state saying so MUST be published for the
  whole time it is outstanding. "Able to show" describes a capability rather than a behaviour,
  and a capability nothing exercises is not observable; a wait the developer cannot see is
  indistinguishable from a frozen window.
- **FR-021c**: A confirmation that does not complete within its limit MUST end the wait, report
  that the content could not be verified, and offer the cached copy marked as unverified. An
  unbounded wait is not an option, because the engine being wedged is exactly when the cache is
  most useful.
- **FR-022**: Cached content MUST survive a rename **performed through the provider**: the
  projection MUST identify a file by an opaque identity rather than by its path, and MUST expose an
  operation that moves a file's path while leaving its content untouched.
- **FR-022a**: A rename **discovered by re-listing a folder** loses that file's cached content, and
  the next open refetches it. This is an accepted limit, not a defect. Within this feature a
  re-listing sees only that one name vanished and another appeared; nothing links the two without
  hashing every entry, which costs more than the refetch it would save. The requirement that makes
  FR-022 pay — a rename the client itself performed — arrives with the write path in F006, which is
  the first feature with a caller for the operation FR-022 requires.
- **FR-022b**: Losing cached content to a rename MUST NOT lose the file. This is the same guarantee
  FR-027 makes for eviction, under a different trigger: content can go, the tree entry stays listed
  and navigable. Stated in both places because the two causes are found in different code, and an
  implementation can satisfy one while breaking the other.

**Size**

- **FR-023**: A file too large for a single message MUST be readable in ranges, and the developer
  MUST see the beginning without waiting for the whole.
- **FR-024**: A directory listing MUST be returned in pages of at most 1000 entries, and a
  caller MUST be able to request the next page. "Bounded" is not a testable bound — a listing of
  a hundred thousand entries at roughly a hundred bytes each exceeds §4.1's frame cap by an order
  of magnitude, so a single-message listing is not merely slow but undeliverable.

  **This requires adding pagination to `workspace/readDirectory` in §4.8**, which the catalogue
  does not currently describe. Recorded here as a consequence rather than discovered during
  implementation, the way `session/onRestart` was in F002.
- **FR-025**: Content that would not fit in a single protocol message MUST travel beside the
  channel rather than through it, per A-BULK. The threshold is **derived from** the frame cap in
  §4.1 and stated as a byte count in the plan, so the rule is decidable from the content's size
  rather than from a judgement about what counts as bulk. It cannot **equal** the cap: content is
  carried encoded, and the encoding expands it, so a raw threshold set at the cap produces a
  message that exceeds it.

**Retention**

- **FR-026**: Content unopened for fourteen days MUST be removed.
- **FR-026a**: Eviction MUST run once at startup, before any workspace is opened, and MUST NOT
  run while a workspace is in use. Deleting competes with reading on the same store, and the one
  thing §1.4 protects is a sidebar that answers in under a millisecond — an eviction holding a
  write lock is precisely what breaks that.
- **FR-026b**: A session that is never restarted therefore never evicts, and disk may grow for
  its duration. This is an accepted limit rather than an oversight: the alternative runs eviction
  while somebody is working.
- **FR-027**: Eviction MUST remove content only. The tree MUST remain navigable and the file
  MUST remain listed. FR-022b requires the same of the other way content is lost.
- **FR-028**: Every cache hit MUST record that access, so retention measures use rather than age.
- **FR-029**: Evicted content MUST be re-fetchable without the developer being told anything
  unusual happened.
- **FR-030**: Cached content MUST be stored compressed, and its hash MUST be computed over the
  uncompressed bytes so it is directly comparable with the engine's.

**Latency**

- **FR-036**: Expanding a folder whose contents are already cached MUST complete within 1 ms,
  and one that requires a fetch within 250 ms, per §1.4. Principle V makes the interaction budget
  a measured question rather than an argued one, and A-NFR fixes how: p99 at the boundary
  between the interface and the transport, with any harness delay excluded.
- **FR-037**: Opening a cached file while connected costs a confirmation round trip by
  FR-021a, so the cache MUST NOT be claimed to make opening faster while online. Its online value
  is transfer avoided; its offline value is availability. Stating this prevents a later
  measurement being read as a regression against a promise nobody made.

**Offline**

- **FR-031**: While disconnected, path search MUST be served from the projection without a
  request being attempted.
- **FR-032**: While disconnected, cached content MUST be served and MUST be presented as possibly
  stale rather than as current.
- **FR-033**: While disconnected, uncached content MUST be reported as unavailable rather than
  shown as empty.

**Failure**

- **FR-034**: Failing to cache MUST NOT fail the read that prompted it. Caching is an
  optimisation.
- **FR-035**: Every behaviour here MUST be verifiable with no remote host and no network,
  consistent with the standard F001 established and A-TEST made binding.
- **FR-038**: A workspace whose root no longer exists on the engine MUST be reported as gone, and
  its projection MUST stop being presented as a live view. The developer MUST be able to tell the
  difference between a folder that has not been fetched yet and a workspace that is not there any
  more — browsing a projection of something deleted is the one case where the cache silently
  becomes fiction rather than a stale fact.

### Key Entities

- **Workspace**: A registered root the developer browses. Has a stable identity independent of
  its display name, a location, and a projection of its own.
- **File entry**: One node in the tree — its name, whether it is a directory, its parent, its
  size, and whether content for it is cached. Identified by an opaque identity so a rename does
  not discard its content.
- **Cached content**: Compressed bytes for one file entry, with the hash of the *uncompressed*
  content and the time it was last served.
- **Path**: A workspace-relative location, validated on construction and asserted to be a
  descendant of its workspace root before any filesystem operation.
- **Byte range**: An offset and length within a file, so a large file can be read in parts.
- **Schema version**: What shape a projection on disk is in, so an older one is migrated or
  discarded rather than misread.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Opening a workspace of any size fetches exactly one directory listing.
- **SC-002**: The number of listings requested equals the number of folders the developer
  expanded, across a session of arbitrary length.
- **SC-003**: Re-opening an unchanged file transfers zero bytes of content.
- **SC-004**: A file whose remote content changed is never served from the cache — zero
  occurrences across every exercised change scenario.
- **SC-004a**: While connected, unverified content reaches the developer zero times.
- **SC-004b**: A confirmation that never arrives ends within its limit and produces a stated
  outcome, in 100% of exercised cases — never an indefinite wait.
- **SC-004c**: Expanding a cached folder completes within 1 ms and an uncached one within 250 ms
  at the 99th percentile, with the measured values printed rather than only compared.
- **SC-005**: A file modified in git but unchanged in content is served from the cache in 100%
  of cases.
- **SC-006**: A path escaping the workspace root is refused by the engine in 100% of cases,
  including when the client sent it without checking.
- **SC-007**: Two workspaces with identical display names each read back their own content, with
  zero cross-reads.
- **SC-008**: Content unopened past the retention window is removed, while 100% of tree entries
  remain listed and navigable.
- **SC-008a**: Zero evictions occur while a workspace is open, across a sustained working
  session.
- **SC-009**: An evicted file re-opens successfully with no error surfaced to the developer.
- **SC-010**: Cached content occupies at most half the disk of the content it represents, across
  a representative source tree, with the achieved ratio printed rather than only compared — a
  budget only ever compared against tells nobody how much headroom is left (A-NFR).
- **SC-011**: While disconnected, a path search over a populated projection returns results with
  zero requests attempted.
- **SC-012**: While disconnected, opening an uncached file produces a stated reason rather than
  an empty document, in 100% of cases.
- **SC-013**: A cache written by an older schema is migrated with its content preserved, and is
  never read with the wrong shape at any point.
- **SC-013a**: A migration publishes its running state at least once per second for its whole
  duration, in 100% of exercised upgrades — asserted on the number and spacing of those reports,
  not on one having been sent.
- **SC-013b**: A failed migration leaves zero readable half-transformed projections across every
  exercised failure mode, and the developer is told the cache was rebuilt.
- **SC-014**: The full suite for this feature runs with no remote host and no network.
- **SC-015**: A workspace deleted on the engine while open is reported as gone in 100% of exercised
  cases, with zero instances of its projection continuing to be presented as current.

## Assumptions

- **The engine is deployed and a session exists.** F002 delivers that, and this feature consumes
  it rather than re-establishing it.
- **The remote workspace already exists on the host.** Provisioning one is not this feature's
  concern; A-WORKSPACE governs naming, collision and deletion.
- **Content search is the engine's job.** The client never indexes (§10), so offline search is
  limited to paths already known to the projection.
- **Retention is measured in days, not size.** A-EC2 makes the instance single-tenant and the
  developer's own, so a disk budget is theirs to manage; fourteen days is the stated rule and no
  size cap is imposed on top of it.
- **F002's threat model still holds.** The remote host is inside the developer's trust boundary.
  Path containment is not defence against a hostile host — it is defence against a malformed or
  stale client, which is what Principle VI names.
