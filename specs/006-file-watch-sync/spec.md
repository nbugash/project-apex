# Feature Specification: File Watch Sync

**Feature Branch**: `feature/F004-file-watch-sync`

**Created**: 2026-09-24

**Status**: Draft

**Input**: Feature map entry F004 `file-watch-sync` — the engine-side watcher scoped to the
workspace, the configurable ignore set shared with the indexer, `workspace/onFileEvent` emission
with event coalescing, client cache invalidation and open-file refresh, and bulk `invalidateAll`
handling for large changes.

**Terminology**: the *engine* is the remote process (`ide-engine`, §4.8). The *projection* is the
client's local cache of a workspace, delivered by F003. A *watch* is the engine observing a
workspace for change; an *event* is what it sends when something changes.

## On the source of values

Every normative value here comes from the system specification or a recorded decision. Where this
document states a rule it is quoting one.

- **§10.3** — the engine owns all file watches, scoped to the workspace. **The client sets no OS
  watches in remote mode**; it receives events and acts only on paths it is currently displaying.
- **§10.4** — operations that change many files at once emit `workspace/invalidateAll` rather than
  thousands of individual events. The client marks its tree stale and re-queries lazily.
  **Invalidation of the tree is not invalidation of content**: blobs remain valid or not on their
  own hash terms.
- **§5.3 / FR-019 (F003)** — a cached blob is valid exactly when its hash matches the engine's.
  Nothing else invalidates it. A file event is not a hash.
- **A-IGNORE** — one resolved exclusion set per workspace, the repository's `.gitignore` files plus
  a fixed built-in set, used by **both** the indexer and the watcher. No per-workspace user
  configuration in v1.
- **§4.6** — one pipe is one queue. Nothing may flood the control channel and delay interactive
  traffic.
- **§1.4** — the interaction budget. A change arriving from elsewhere must not stall the interface.
- **Constitution Principle VI** — every path crossing the boundary is untrusted at the receiving
  end. An event carries a path, and the client is the receiving end.

## What this feature is not

**It is not search.** The exclusion set is shared with the indexer (A-IGNORE) and this feature
computes it, but indexing and content search belong to F013.

**It does not write.** Nothing here creates, modifies or deletes a file. It observes. The write
path is F006's, and the events this feature delivers are what tell a client that someone *else*
wrote.

**It does not merge.** A file changing underneath an open editor is reported; deciding what the
editor does about unsaved local edits is F006's, and reconciling divergent offline edits is
F012's and F019's.

**It does not watch in local mode.** §10.3 scopes watching to the engine. Local Mode (F015) runs
without one and will need its own answer; this feature does not provide it.

**It does not make the cache authoritative.** An event says something changed, not what it now
is. Content validity remains a hash comparison (§5.3), and this feature never marks a blob valid.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See a colleague's change without asking for it (Priority: P1)

A developer has a repository open. Someone else — a colleague pushing to the same checkout, a
build writing an artifact, a script generating code — changes a file on the remote host. The tree
and the affected file reflect it without the developer refreshing anything.

**Why this priority**: This is the feature. Without it the projection is only ever as fresh as the
last thing the developer happened to open, and a cache that silently serves yesterday's tree is
worse than no cache — it is confidently wrong.

**Independent Test**: Open a workspace, change a file on the host by other means, and confirm the
interface reflects it within the stated interval with no developer action.

**Acceptance Scenarios**:

1. **Given** a folder the developer is viewing, **When** a file is created in it on the host,
   **Then** the new file appears in the tree.
2. **Given** a folder the developer is viewing, **When** a file in it is deleted on the host,
   **Then** it disappears from the tree.
3. **Given** a file whose content is cached, **When** it changes on the host, **Then** the next
   read serves the new content rather than the cached copy.
4. **Given** a folder the developer has never expanded, **When** files change inside it, **Then**
   nothing is fetched — the change is noted, not chased.
5. **Given** an excluded directory such as `node_modules`, **When** files change inside it,
   **Then** no event is delivered at all.

---

### User Story 2 - Survive a branch switch without a flood (Priority: P1)

A developer switches branches, pulls, or runs a build that rewrites thousands of files. The
interface stays responsive throughout and shows the new state.

**Why this priority**: P1 because this is the case that breaks a naive watcher, and it is routine
rather than exceptional. §4.6 makes one pipe one queue: thousands of individual events would
serialise ahead of every interactive request, so the feature that adds watching is the feature
that can destroy the interaction budget.

**Independent Test**: Change several thousand files at once on the host and confirm the interface
remains responsive and converges on the new state.

**Acceptance Scenarios**:

1. **Given** a change affecting more files than the per-change limit, **When** it happens,
   **Then** a single wholesale invalidation is delivered instead of individual events.
2. **Given** a wholesale invalidation, **When** it arrives, **Then** the tree is marked stale and
   re-queried only as the developer navigates, not all at once.
3. **Given** a wholesale invalidation, **When** it arrives, **Then** cached content is **not**
   discarded — each blob remains valid or not on its own hash terms.
4. **Given** any burst of changes, **When** it is in progress, **Then** interactive actions
   continue to meet the interaction budget.

---

### User Story 3 - Know that the file you are reading has moved on (Priority: P1)

A developer is looking at a file. It changes on the host. They are told, rather than continuing to
read something that is no longer true.

**Why this priority**: P1 because the alternative is the specific failure the cache was built to
avoid. A developer acting on stale content they believe is current is worse off than one who knows
they are offline.

**Independent Test**: Open a file, change it on the host, and confirm the developer is told without
the file being altered underneath them.

**Acceptance Scenarios**:

1. **Given** a file the developer has open, **When** it changes on the host, **Then** they are
   told it has changed.
2. **Given** a file the developer has open with no unsaved local changes, **When** it changes on
   the host, **Then** the new content can be shown.
3. **Given** a file the developer is **not** looking at, **When** it changes, **Then** they are not
   interrupted.
4. **Given** a file that is renamed on the host, **When** the event arrives, **Then** the tree
   shows it at its new path.

---

### User Story 4 - Trust that watching stopped when it should (Priority: P2)

A developer closes a workspace, loses their connection, or the engine restarts. Watching stops
cleanly and resumes when it should, without leaking resources on the host or leaving the client
believing it is being told about changes when it is not.

**Why this priority**: P2 because it does not affect the common path, and it is what makes the
feature survive a long session. A watch that outlives its workspace consumes a scarce host
resource; a client that believes it is watching when it is not shows a tree that stopped updating
and says nothing.

**Independent Test**: Open and close workspaces repeatedly and confirm host watch resources return
to their starting level; drop the connection and confirm the developer learns that changes are no
longer being reported.

**Acceptance Scenarios**:

1. **Given** a workspace being closed, **When** it closes, **Then** its watches are released on
   the host.
2. **Given** a lost connection, **When** it drops, **Then** the developer is told that changes are
   no longer being reported rather than seeing a tree that has quietly stopped updating.
3. **Given** a reconnection, **When** it completes, **Then** the tree is brought up to date,
   because anything that changed while disconnected was never delivered.
4. **Given** an engine that restarted, **When** the client re-attaches, **Then** watching resumes
   and the client is told that events were missed.

---

### Edge Cases

- **A change inside an excluded directory.** `node_modules` churns constantly during an install.
  No event may be delivered, and the exclusion must be the same set the indexer uses, or search
  returns results for files whose changes are never noticed.
- **A file changed thousands of times in a second.** A compiler writing output, a log being
  appended. The developer needs to know it changed, not to be told once per write.
- **A rename.** Arrives as one event naming both paths, not as a delete and an unrelated create —
  the tree must move the entry rather than losing and re-finding it.
- **A directory renamed with a large subtree inside it.** Every descendant's path changes at once,
  and none of them were individually touched.
- **A change arriving for a path the client has never fetched.** Nothing to update, and fetching it
  would defeat §10.1's whole approach.
- **A change arriving for a path outside the workspace root.** A symlinked directory, or a
  malformed event. The client must not act on it.
- **The host's watch capacity is exhausted.** Observing a filesystem costs a resource the host
  limits per user, and a large repository can exhaust it. The developer must learn that changes
  are not being reported rather than believing a silent watcher is a quiet repository.
- **The workspace root is deleted while watched.** Distinct from a file inside it disappearing.
- **Events arriving during a reconnection.** The window between the connection returning and the
  watch being re-established, where changes happen and nobody is listening.
- **An event for a file whose cached content is already stale.** The hash already disagrees; the
  event must not make the situation worse or cause a double fetch.
- **The engine restarting.** Watches live in the engine's memory; a restart loses them, and the
  client must not assume silence means stability.

## Requirements *(mandatory)*

### Functional Requirements

**Watching**

- **FR-001**: The engine MUST observe the workspace for changes. The client MUST NOT set operating
  system watches in remote mode (§10.3).
- **FR-002**: Watching MUST be scoped to the workspace root. A change outside it MUST NOT produce
  an event.
- **FR-003**: Watching MUST begin without the developer asking for it. A registered, open workspace
  is a watched one.
- **FR-004**: Watches MUST be released when the workspace closes, when the connection drops, and
  when the engine exits.
- **FR-005**: The system MUST report when it cannot watch — including when the host's watch
  capacity is exhausted — rather than appearing to watch and delivering nothing. Silence MUST NOT
  be the way a developer discovers that watching failed.

**Exclusions**

- **FR-006**: One exclusion set MUST be computed per workspace, from the repository's own
  `.gitignore` files plus the fixed built-in set named in A-IGNORE.
- **FR-007**: That set MUST be the same one the indexer uses. An indexer and watcher that disagree
  produce search results for files whose changes are never noticed (A-IGNORE).
- **FR-008**: No event MUST be delivered for an excluded path.
- **FR-009**: There MUST be no per-workspace user configuration of exclusions in this version
  (A-IGNORE).

**Events**

- **FR-010**: An event MUST identify its workspace, what happened, and the path it happened to.
- **FR-011**: A rename MUST arrive as a single event naming both the old and the new path, not as
  a deletion followed by an unrelated creation.
- **FR-012**: Repeated changes to one path MUST collapse into one event rather than one per change.
  The collapsing window MUST be **a stated duration fixed in the plan**, not a judgement made per
  event — "coalesce when there are a lot" is not implementable and not testable. A file written a
  thousand times in a second is one thing a developer needs to know about, and the requirement is
  that the count of events delivered is bounded by elapsed time rather than by writes.
- **FR-013**: An event MUST NOT carry file content. It says something changed, not what it now is.
- **FR-014**: The client MUST treat every path in an event as untrusted and MUST refuse one that
  escapes the workspace root, independently of anything the engine checked (Principle VI).

**Volume**

- **FR-015**: A change affecting more paths than a threshold MUST be delivered as a single
  wholesale invalidation rather than as individual events (§10.4). The threshold MUST be **a stated
  count fixed in the plan**, so the rule is decidable from the number of affected paths rather than
  from a judgement about what counts as bulk. §10.4 names the cases — branch switches, large pulls
  — but no number, and "thousands" is not a bound a test can assert against.
- **FR-016**: Event delivery MUST NOT delay interactive traffic. The channel carries control
  traffic, and one pipe is one queue (§4.6).
- **FR-017**: A wholesale invalidation MUST mark the tree stale and MUST NOT trigger a refetch of
  anything the developer is not looking at. Re-query happens lazily, as they navigate (§10.4).

**Effect on the projection**

- **FR-018**: A wholesale invalidation MUST NOT discard cached content. Tree invalidation is not
  content invalidation; each blob remains valid or not on its own hash terms (§10.4, §5.3).
- **FR-019**: An event MUST NOT mark any cached content **valid**. Validity is a hash comparison
  and nothing else (§5.3). An event may only cause content to be re-checked or discarded.
- **FR-020**: An event for a path the client has never fetched MUST NOT cause it to be fetched.
- **FR-021**: A rename event MUST move the tree entry to its new path rather than removing and
  re-adding it, so cached content survives (F003's FR-022).
- **FR-022**: A directory rename MUST be reflected for every descendant path, none of which was
  individually changed.

**What the developer sees**

- **FR-023**: A file the developer is currently viewing MUST be reported as changed when it changes
  on the host.
- **FR-024**: A change to a file the developer is **not** viewing MUST NOT interrupt them.
- **FR-025**: While changes are not being reported — disconnected, or watching unavailable — the
  developer MUST be told, rather than seeing a tree that has quietly stopped updating.
- **FR-026**: On reconnection the tree MUST be brought up to date, because anything that changed
  while disconnected was never delivered.

**Failure**

- **FR-027**: Losing the ability to watch MUST NOT make the workspace unusable. Browsing and
  reading continue; only automatic freshness is lost, and its loss is stated.
- **FR-028**: Every behaviour here MUST be verifiable with no remote host and no network,
  consistent with the standard F001 established and A-TEST made binding.

### Key Entities

- **Watch**: The engine's observation of one workspace. Has a lifetime bounded by the workspace
  being open, and consumes a host resource that is finite.
- **File event**: What happened, where, and — for a rename — where it went. Carries no content.
- **Exclusion set**: The resolved set of paths not watched and not indexed, computed once per
  workspace from the repository's own ignore files plus a fixed built-in set.
- **Staleness**: A property of a tree region, meaning "re-query this before trusting it". Distinct
  from content validity, which is a hash comparison.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A change to a file the developer is viewing is reflected within 2 seconds of it
  happening on the host, at the 99th percentile.
- **SC-002**: A change inside an excluded directory produces zero events, across every exercised
  exclusion.
- **SC-003**: The watcher's exclusion set and the indexer's are identical in 100% of exercised
  workspaces — compared as sets, not asserted.
- **SC-004**: A change affecting more paths than the limit produces exactly one wholesale
  invalidation and zero individual events.
- **SC-005**: During a burst of ten thousand changes, interactive actions continue to meet §1.4's
  budget, with the measured value printed rather than only compared.
- **SC-006**: A wholesale invalidation discards zero cached content blobs.
- **SC-007**: A file written one thousand times in one second produces a number of events bounded
  by the elapsed time divided by the collapsing window — never one per write. Asserted on the
  count, with the window's value read from the plan rather than assumed by the test.
- **SC-008**: A rename produces exactly one event naming both paths, and the cached content of the
  renamed file survives in 100% of exercised cases.
- **SC-009**: Closing a workspace returns host watch resources to their pre-open level, with zero
  leaked watches across a hundred open-and-close cycles.
- **SC-010**: An event naming a path outside the workspace root is refused by the client in 100% of
  cases, including when the engine sent it.
- **SC-011**: When watching is unavailable, the developer is told in 100% of exercised cases — zero
  instances of a silently non-updating tree.
- **SC-012**: A change made while disconnected is reflected after reconnection in 100% of exercised
  cases.
- **SC-013**: The full suite for this feature runs with no remote host and no network.

## Assumptions

- **The workspace is registered and the projection exists.** F003 delivers both, and this feature
  observes what it already holds rather than building a second view.
- **The engine is the only writer this feature learns about.** Writes the client itself performs
  arrive through F006's write path; this feature reports what changed *elsewhere*.
- **Coalescing loses no information that matters.** A developer needs to know a file changed, not
  how many times. The interval is a value to be chosen, and the requirement is that repeated
  changes to one path collapse.
- **Two seconds is this specification's own choice**, not a value quoted from §1.4. §1.4 budgets
  interactions the developer initiates; a change arriving from elsewhere is not one, so it has no
  existing target. Two seconds is chosen as the point past which a developer starts to wonder
  whether the system noticed — fast enough to feel live, slow enough to leave room for collapsing
  repeated writes. It is stated here so a later measurement is read against a number somebody
  chose rather than one that appeared.
- **The host's watch capacity is finite and shared.** The resource is limited per user rather than
  per workspace, so a large repository can exhaust it, and the system must degrade legibly rather
  than silently.
- **No per-workspace exclusion configuration.** A-IGNORE declined it for v1, and this feature does
  not reopen that.
- **F002's threat model still holds.** The remote host is inside the developer's trust boundary.
  Refusing an out-of-root path in an event is defence against a malformed or stale engine, which is
  what Principle VI names.
