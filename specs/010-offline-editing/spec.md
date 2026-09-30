# Feature Specification: Offline Editing

**Feature Branch**: `feature/F012-offline-editing`

**Created**: 2026-09-27

**Status**: Draft

**Input**: F012 `offline-editing` — five subfeatures from `specs/features-map.md`: offline UI
state consuming F001's connection state rather than re-detecting it; cached-only tree, file and
path-search behaviour; background prefetch of recent-commit and manifest files; local
persistence of offline edits against their base revision; and reconnection fast-forward where
the remote has not moved.

## Clarifications

### Session 2026-09-27

- Q: §11.1 to §11.3 of the system specification still describe the read-only offline lock that
  A-OFFLINE reversed, while §11.5 describes the merge. Where should that be corrected? → A:
  **In the system specification.** First answered "in this feature's spec, not the system
  spec"; `/speckit-analyze` then found that Principle II forbids exactly that resolution — "where
  any other document contradicts it, the contradiction MUST be resolved in that file before
  dependent work starts" — and raised it CRITICAL. §11.1, §11.2, §11.3, §11.5's step 5, §7's
  citing line and §13.1 are rewritten, and §5.2 gains the `pending_edits` table A-OFFLINE's
  Consequences called for. **Reversed 2026-09-27.** The first answer was made against a
  recommendation that argued discoverability rather than the rule; the rule was the answer, and
  reading Principle II before offering the option would have avoided the round trip.
- Q: A-OFFLINE says F012 "grows from one feature to roughly three". How much does this cycle
  build? → A: **All five subfeatures in one cycle**, as the feature map carries them. The
  alternative was splitting along A-OFFLINE's estimate and shipping the merge engine separately.
  **The known cost:** the merge semantics — the part of this product that can destroy a
  colleague's work — arrive at the end of the largest change set the project has produced, and
  a reviewer's attention is finite. The mitigation is that the merge is specified, planned and
  reviewed as its own user story with its own acceptance scenarios, so it can be read alone.

- Q: When a file changed both offline and on the host, what counts as "overlapping" — the thing
  that makes the client prompt rather than merge silently? → A: **Git's own merge semantics** — a
  context-aware three-way merge, the way `git merge-file` behaves, so changes near each other
  conflict even when they touch no identical line. A-OFFLINE justifies prompting on the grounds
  that developers "already have an accurate mental model for this from version control"; that
  model is git's merge, so matching it is what makes the mental model transfer. Rejected:
  overlapping line ranges with zero context, which combines two edits to adjacent lines — a
  signature split across two lines, one side changing each — into something neither person wrote.
- Q: Offline, is a developer's work retained when they save, or continuously as they type? → A:
  **On save.** An unsaved buffer offline behaves exactly as it does online: not written anywhere,
  lost if the application dies. Keeps one meaning for "saved", avoids a second store and a second
  lifecycle, and makes SC-002 unambiguous. The accepted cost is that a crash mid-typing loses the
  same work it would lose online, which some developers will expect an offline mode to protect.
- Q: What happens when a developer edits a file offline that cannot be merged line by line — a
  binary, or one above the editor's size limit? → A: **It is editable, and it always prompts on
  reconnection**, whether or not the host changed it. Never silently wrong, and it imposes no new
  restriction on what the developer may edit. The accepted cost is a prompt on files nobody else
  touched: the client cannot merge them, so it will not claim to have.
- Q: How much may the background prefetch cache before going offline? → A: **Bounded, and it
  never evicts to make room.** A fixed number of recent commits plus the project manifests, and
  it stops when continuing would displace something. Speculative work must not push out content
  the developer actually opened, which would make the cache worse at its primary job to be better
  at a guess. The accepted cost is that prefetch may complete only partially on a large
  repository, and partial completion is a normal outcome rather than a failure.

### System-specification subsections this feature corrected

A-OFFLINE (2026-09-23) reversed A-B3 and its Consequences state that §11 is rewritten. Only
§11.5 had been updated; the rest were corrected as part of this feature, after analyze raised the
contradiction as a Principle II violation.

| Subsection | What it said | What it says now |
|---|---|---|
| §7 | "read-only access when disconnected" | read **and** write access, with saved work held until reconciled |
| §11.1 | "read-only mirror", "nothing is queued", headed "Decision (A-B3)" | Headed "Decision (A-OFFLINE)"; the editor stays writable, and only file content is ever held |
| §11.2 | "Monaco models switch to `readOnly: true`" | The editor stays writable; a file with work not yet on the host is shown as held locally |
| §11.3 | Editor offline: "Cached files only, read-only" | Cached files only, **writable**, held locally until reconciled. The table is also now the enumeration FR-004 is bound to |
| §11.5 step 5 | "Unlock the editor" | Reconcile per file; nothing was locked |
| §13.1 | "An offline workspace ... is read-only" | Readable and writable, with saved work awaiting reconnection |
| §5.2 | No table for offline work | `pending_edits` added (A-PENDING). `git_status` also corrected to the shape F011 shipped, which had not been propagated either |

- Q: What starts prefetch? → A: A workspace opened while connected, and a transition into
  connected. Not idleness. Found by analyze run 19: every artifact described what prefetch does
  when it runs and none said what runs it, which is the shape F011 shipped three times — an
  emitter with no subscriber, a subscriber with no emitter, a `translate()` with no caller.
  Scenario 1 had said "connected and idle", which implies a detector §4.6 makes redundant:
  interactive traffic already wins the race to the wire in both directions, and the starvation
  that implies is already bounded. A second mechanism for that ordering is what Principle II
  refuses. **Rejected — also prefetch when HEAD moves**: the recent-commit set does go stale
  during a long session, and A-GITNUDGE is the precedent for a second trigger, but it couples
  prefetch to F011's watch and re-runs on every commit in a busy repository. Left to the
  reviewer as an enhancement rather than taken silently.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Know I am offline, and keep reading (Priority: P1)

A developer's connection drops — a train tunnel, a closed laptop lid, a dead VPN. The interface
says so plainly. Their open tabs stay open, the tree stays navigable, and every file they have
already opened is still readable. Things that genuinely need the engine say they need the engine
rather than appearing broken or hanging.

**Why this priority**: Without this, every other story is invisible. It is also the story that
decides whether offline feels like a working tool or a broken one, and that judgement is made in
the first five seconds.

**Independent Test**: Open several files, cut the connection, and confirm the interface says it
is offline, every opened file still reads, and no action hangs waiting for a reply that is not
coming.

**Acceptance Scenarios**:

1. **Given** a connected workspace with files open, **When** the connection drops, **Then** the
   status bar shows a distinct offline state within two seconds and no tab closes or resets.
2. **Given** an offline workspace, **When** the developer opens a file that was cached,
   **Then** its content appears without any request being issued.
3. **Given** an offline workspace, **When** the developer opens a folder that was never listed,
   **Then** it is marked unavailable rather than shown empty.
4. **Given** an offline workspace, **When** the developer runs a path search, **Then** results
   come from cached paths and the interface does not claim the results are complete.
5. **Given** an offline workspace, **When** the developer attempts a content search, a terminal,
   or a code-intelligence action, **Then** each states that it requires the engine.
6. **Given** an offline workspace, **When** the developer looks at the tree, **Then** git state
   is the last known state and is marked as such rather than cleared.

---

### User Story 2 - Keep editing, and lose nothing (Priority: P1)

**Builds on US1.** Retaining an edit requires knowing the client is offline, which US1 is where the client learns. Independently *testable*, not independently deliverable.

The developer keeps working. They edit files, save them, close tabs, and reopen them. Nothing
tells them to stop, and nothing is silently discarded — including across a restart of the
application while still offline.

**Why this priority**: This is the feature. A-OFFLINE reversed the read-only decision precisely
because product judged offline editing a differentiator worth paying for, and an offline editor
that loses work is worse than one that refuses it.

**Independent Test**: Offline, edit and save several files, quit the application, relaunch it
still offline, and confirm every edit is present.

**Acceptance Scenarios**:

1. **Given** an offline workspace, **When** the developer edits a cached file, **Then** the
   editor accepts the edit and does not present itself as read-only.
2. **Given** an offline edit, **When** the developer saves, **Then** the edit is retained locally
   against the content the client last confirmed with the host, and the interface says the work
   is held locally rather than written to the host.
3. **Given** offline edits to several files, **When** the application is quit and relaunched
   while still offline, **Then** every **saved** edit is still present and still marked as held
   locally, and an unsaved buffer is gone exactly as it would be online.
4. **Given** an offline edit, **When** the developer reopens that file in a new tab, **Then**
   they see their edited content, not the last content fetched from the host.
5. **Given** a file the developer creates while offline, **When** they save it, **Then** it is
   retained with no base content, because there is nothing it differs from.
6. **Given** an offline edit, **When** local storage cannot accept it, **Then** the developer is
   told before they lose the buffer, rather than after.

---

### User Story 3 - Reconnect without merging by hand (Priority: P2)

**Builds on US2.** There is nothing to reconcile until something has been retained.

The connection returns. Where nobody else touched the file, the developer's offline work simply
goes to the host. Where somebody did, but in a different part of the file, it still goes —
merged. The developer is not asked to adjudicate changes that do not actually disagree.

**Why this priority**: Fast-forwarding is what makes offline editing usable rather than merely
possible. A reconnection that asked about every file would be a worse experience than the
read-only lock it replaced.

**Independent Test**: Make offline edits, change unrelated files and unrelated regions on the
host, reconnect, and confirm everything lands with no prompt.

**Acceptance Scenarios**:

1. **Given** offline edits and a host where those files have not changed, **When** the connection
   returns, **Then** the edits reach the host with no developer interaction and the files stop
   being marked as held locally.
2. **Given** an offline edit and a host change to a different region of the same file, **When**
   the connection returns, **Then** both changes are present afterwards and the developer is not
   prompted.
3. **Given** offline edits, **When** the connection returns, **Then** a file that reached the
   host is no longer shown as held locally, and one that did not still is.
4. **Given** a reconnection in progress, **When** the connection drops again mid-way, **Then**
   no file is left partly written and every unreconciled edit is still held locally.
5. **Given** offline edits, **When** the connection returns, **Then** the developer is shown what
   happened to their work rather than having to infer it from the tree.

---

### User Story 4 - See a real conflict, and decide it myself (Priority: P2)

**Builds on US3.** A conflict is an outcome of reconciliation, so there is no conflict to show without it.

Two people changed the same lines. The client does not choose. It shows the developer what they
wrote, what the host has, and what they started from, and waits.

**Why this priority**: The remote side is where CI and colleagues write. A merge that silently
picks a side is a merge nobody can audit, and this is the one failure this product cannot
afford. It is P2 only because a conflict is rarer than a clean reconnection, not because it
matters less.

**Independent Test**: Edit the same lines offline and on the host, reconnect, and confirm the
developer is asked and that neither version is written until they answer.

**Acceptance Scenarios**:

1. **Given** an offline edit and a host change to the **same** lines, **When** the connection
   returns, **Then** the developer is prompted and neither version has been written to the host.
2. **Given** a conflict, **When** the developer has not yet answered, **Then** their offline work
   is still held locally and nothing has been discarded.
3. **Given** a conflict, **When** the developer chooses a resolution, **Then** that resolution is
   written to the host and the file stops being marked as held locally.
4. **Given** a conflict the developer leaves unresolved, **When** they go offline again, **Then**
   the conflict and both versions survive, and they are presented again on the next reconnection.
5. **Given** a file that was deleted on the host while it was edited offline, **When** the
   connection returns, **Then** the developer is asked rather than the deletion or the edit
   winning silently.
6. **Given** an offline edit to a file the client cannot merge line by line, **When** the
   connection returns and the host has **not** changed it, **Then** the developer is still
   prompted, because the client cannot combine such a file and must not appear to have decided.
7. **Given** two offline edits in different regions of one file, **When** the host changed a
   third region close to one of them, **Then** the developer is prompted for that region on the
   terms a version-control merge would use, not for the file as a whole.

---

### User Story 5 - Have what I need before I lose the connection (Priority: P3)

**Independent of US1 through US4.** Could be built first or last, and is the only story besides
US1 that is independently deliverable as well as independently testable.

The client caches deliberately while online, so that going offline is not a lottery about which
files happen to be in the cache.

**Why this priority**: It changes offline from "the files I happened to open" to "the files I am
likely to want". Valuable, and the only story here whose absence degrades rather than breaks the
feature.

**Independent Test**: Work online for a while without opening the project's manifests or recently
changed files, go offline, and confirm those files are readable.

**Acceptance Scenarios**:

1. **Given** a workspace opened while connected, **When** prefetch begins, **Then**
   files changed in recent commits and the project's manifest files are cached without the
   developer asking.
2. **Given** prefetch in progress, **When** the developer performs any interactive action,
   **Then** that action is not delayed by the prefetch.
3. **Given** a workspace with no repository, **When** prefetch runs, **Then** manifests are still
   cached and nothing fails.
4. **Given** prefetch running, **When** the connection drops, **Then** whatever completed is
   usable and the partial work leaves no half-written cache entry.
5. **Given** a cache with no room left, **When** prefetch would need to displace cached content
   to continue, **Then** it stops, displaces nothing, and this is not reported as a failure.

---

### Edge Cases

Each case carries an identity so a task can cite it by name. Citing by position
would mean every insertion silently reassigns the cases after it.

- **EC-01**: A file is **deleted on the host** while the developer edited it offline. Covered by US4
  scenario 5: the developer is asked.
- **EC-02**: A file is **deleted locally** by the developer while offline: not propagated, returns on the
  next refetch (FR-016a).
- **EC-03**: The **workspace root is gone** when the connection returns — the workspace cannot be
  reconciled at all, and the developer must be told rather than shown an empty tree.
- **EC-04**: The connection **drops during reconciliation**, mid-file or between files (FR-028).
- **EC-05**: The application is **quit during reconciliation** and relaunched.
- **EC-06**: **Two offline sessions back to back** with no successful reconnection between them: the base is
  still the last content confirmed with the host, not the previous offline content (FR-011c).
- **EC-07**: A file is **edited offline and never had a base** because it was created offline:
  retained with no base recorded, and reconciled as a create rather than a merge (FR-014).
- **EC-08**: A file is edited offline whose cached content was **evicted** in the meantime: the merge is
  unaffected, because the base travels with the pending edit rather than being referenced
  (FR-011b).
- **EC-09**: **Binary or very large files** edited offline: editable, never merged, and always prompting on
  reconnection (FR-017a, FR-025a).
- **EC-10**: **Local storage is full** while persisting an offline edit: the save fails and the
  developer is told while the work is still in the buffer (FR-016).
- **EC-11**: An **unsaved buffer** when the application stops offline: lost, exactly as online (FR-011a).
- **EC-12**: **Prefetch meets a full cache** and stops part-way (FR-029a).
- **EC-13**: The **same file is edited offline in two windows** of the application. **N/A**: the application
  is a single window (§2), so there is no second editor to disagree with. Recorded rather than
  dropped, because a later multi-window feature inherits the question.
- **EC-14**: The engine's content for a file changed **and changed back** while offline, so the hashes match
  even though the file was touched: **fast-forward is correct** and needs no special handling.
  Reconciliation compares content, not history, and content is what the developer cares about
  (FR-019). Recorded because the case invites a fix it does not need.
- **EC-15**: The developer **resolves a conflict and the host changes again** before the resolution
  is written: the write is refused for a stale base and becomes a new conflict rather than an
  error, and the host's newer content is never overwritten
  (`contracts/offline-commands.md`, `conflict_resolve` guarantee 2).
- **EC-16**: Reconnection succeeds but the **protocol version is incompatible** (§3.8), so the workspace
  cannot be used even though the link is up.
- **EC-17**: The host changes **between the read and the write of one reconciliation**, so the
  engine refuses the write as stale: reported as a conflict against the host's newer content,
  never as a failure, and the retained work stays (FR-020b).

## Requirements *(mandatory)*

### Functional Requirements

**Knowing, and showing, that the client is offline**

- **FR-001**: The client MUST take its offline state from the connection state F001 already
  publishes, and MUST NOT re-detect the connection by its own means.
- **FR-002**: The client MUST show a distinct offline state, explicitly, rather than letting the
  developer infer it from an action that fails or hangs.
- **FR-003**: Losing the connection MUST NOT close a tab, reset the tree, clear cached content,
  or clear the last known git state.
- **FR-004**: Every capability §11.3's component table marks as unavailable offline MUST state
  that it requires the engine, rather than failing in a way that reads as a defect. The table is
  the enumeration: bounding the requirement to it is what makes it closable, where "every
  capability" was quantified over an open set and could never be shown satisfied.

**Reading offline**

- **FR-005**: A file whose content is cached MUST be readable offline without issuing a request.
- **FR-006**: A folder that has been listed MUST stay navigable offline; one that has never been
  listed MUST be marked unavailable rather than shown as empty.
- **FR-007**: Path search MUST work offline over cached paths, and MUST NOT present its results
  as a complete search of the workspace.
- **FR-008**: Content search MUST be unavailable offline and MUST say so.
- **FR-009**: Git state MUST remain readable offline as the last known state, marked as
  unconfirmed rather than cleared.

**Editing offline**

- **FR-010**: The editor MUST remain writable when the client is offline.
- **FR-011**: A **saved** offline edit MUST be retained locally, against the content the client
  last confirmed with the host, identified by that content's hash.
- **FR-011a**: An **unsaved** buffer MUST behave offline exactly as it does online: it is not
  retained, and it is lost if the application stops. Offline is not a stronger promise about
  unsaved work than online is.
- **FR-011b**: The client MUST retain the base **content** alongside its hash, not only the hash.
  Reconciliation compares three versions and needs the base text; the only other copy of it is
  the cache, which is evicted and overwritten on its own terms.
- **FR-011c**: The base MUST be recorded once, when a path first gains offline work, and MUST NOT
  change on a later offline save of the same file. A base re-derived from the newer local content
  would make reconciliation compare local against local — a wrong answer rather than an error.
- **FR-012**: A retained offline edit MUST survive the application being quit and relaunched
  while still offline.
- **FR-013**: A file opened after being edited offline MUST show the developer's edited content,
  not the last content fetched from the host.
- **FR-014**: A file created offline MUST be retained with no base content recorded.
- **FR-015**: The interface MUST distinguish a file whose work is held locally from one whose
  work is on the host.
- **FR-016**: Where an offline edit cannot be retained, the developer MUST be told while the
  work is still in the buffer.
- **FR-016a**: A file the developer **deletes** while offline MUST NOT be propagated to the host
  on reconnection. Only content and its base are held (FR-034), so a deletion is not offline work;
  the file returns on the next refetch, and the developer deletes it again when connected. Stated
  because the alternative — inferring a deletion from an absence — cannot distinguish "deleted"
  from "never cached".
- **FR-017**: Retaining an offline edit MUST NOT change whether cached content is considered
  valid; validity remains a hash comparison and nothing else (§5.3).
- **FR-017a**: A file the client cannot merge line by line — one it does not hold as text, or one
  above the editor's size limit — MUST still be editable offline. It MUST NOT be silently merged
  under any circumstances.

**Reconnecting**

- **FR-018**: On reconnection the client MUST reconcile each file with retained work by comparing
  three versions: the base it recorded, the local content, and the host's current content.
- **FR-018a**: While offline, the client MUST attempt to reconnect on its own, on a bounded,
  jittered backoff, and MUST stop rather than retry where the failure will not fix itself (a
  changed host key, a refused credential). On success it MUST register the current workspace with
  the new engine **before** reconciling, because a reconciliation against an engine that does not
  know the workspace reports every file as failed. Added during implementation: §11.5 specifies
  this loop, and without it nothing reconnected and FR-018 could only be met by relaunching.
- **FR-019**: Where the host's content still matches the recorded base, the local content MUST be
  written to the host without prompting the developer.
- **FR-020**: Where the host's content has changed but does not overlap the local change, the two
  MUST be combined and written without prompting the developer.
- **FR-020a**: Overlap MUST be decided by a **context-aware three-way merge**, matching the
  behaviour of standard version-control merge tools: changes close to one another conflict even
  when they modify no identical line. A merge that combines edits to adjacent lines is not
  permitted, because it can produce a file neither person wrote.
- **FR-020b**: Where the host's content changes **between the read and the write** of one
  reconciliation, so the engine refuses the write with `-32004`, the client MUST treat it as a
  conflict for that file and MUST NOT report it as a failure. The retained work stays (FR-022) and
  the developer is given the three sides against the host's newer content. This is the same answer
  `conflict_resolve` already gives for the same race, and the reason is the same: a stale-base
  refusal is the host disagreeing, which is what a conflict is, while a failure is something the
  developer cannot act on.
- **FR-021**: Where the changes genuinely overlap, the client MUST prompt and MUST NOT write
  either version until the developer decides.
- **FR-022**: Until a file's work has reached the host, that work MUST remain retained; nothing
  may be discarded on the strength of an attempt that did not complete.
- **FR-023**: Reconciliation MUST be per file, so one conflict does not hold back files that have
  no conflict.
- **FR-024**: The developer MUST be shown the outcome of reconciliation rather than having to
  infer it.
- **FR-025**: A conflict left unresolved MUST survive going offline again and MUST be presented
  again on the next reconnection.
- **FR-025a**: A file the client cannot merge line by line MUST prompt on reconnection **whether
  or not the host changed it**. The client cannot combine such a file, so it must not appear to
  have decided anything about it.
- **FR-026**: Where a file was deleted on the host while it was edited offline, the client MUST
  ask rather than resolving it either way.
- **FR-027**: Reconciliation MUST use the base-content hash the write protocol already carries,
  and MUST NOT require a new protocol field.
- **FR-028**: A reconnection interrupted part-way MUST leave no file partly written and MUST
  leave every unreconciled edit retained.

**Prefetch**

- **FR-029**: While connected, the client MUST cache files changed in a bounded number of recent
  commits, and the project's manifest files, without the developer asking.
- **FR-029b**: Prefetch MUST begin on two events and no others: a workspace opened while the
  client is connected, and a transition into the connected state. It MUST NOT wait for the client
  to be idle: §4.6 already orders background work behind interactive work in both directions and
  bounds the starvation that implies, so an idleness detector would be a second mechanism for a
  property the transport already provides.
- **FR-029a**: Prefetch MUST stop rather than evict. It MUST NOT displace cached content to make
  room for speculative content, and stopping early MUST be an ordinary outcome rather than a
  reported failure.
- **FR-030**: Prefetch MUST NOT delay any action the developer initiated.
- **FR-031**: Prefetch MUST leave no partial cache entry when it is interrupted.
- **FR-032**: Prefetch MUST work in a workspace with no repository, caching manifests only.

**What this feature does not do**

- **FR-033**: The client MUST NOT resolve a conflict automatically by preferring either side.
- **FR-034**: The client MUST NOT queue arbitrary host operations for later; what is retained is
  file content and its base, and nothing else.

### Key Entities

- **Pending edit**: a file's locally held content, **the content it was derived from** and that
  content's hash, and when it was retained. Belongs to one workspace and one path. Absent for a file with no
  offline work. The stored thing is always a *pending edit*; "offline edit" is the developer's
  act of making one, and the two are not used interchangeably.
- **Base revision**: the content the client last confirmed with the host for a path, and its
  hash. The hash is already carried by the write protocol; this feature stores both, because a
  three-way merge needs the text and not only the fingerprint.
- **Reconciliation outcome**: per file, what happened on reconnection — written unchanged,
  combined, conflicted, or not yet attempted.
- **Conflict**: the three versions of one file — base, local, host — held until the developer
  decides, and surviving a further disconnection.
- **Prefetch candidate**: a path the client has decided is worth caching before it is asked for.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The offline state is visible within **2 seconds** of the connection dropping.
- **SC-002**: **100%** of **saved** offline edits survive quitting and relaunching the
  application while offline, measured over at least 50 saved edits across at least 10 files.
- **SC-003**: **Zero** offline edits are lost across a full cycle of edit, disconnect, relaunch,
  reconnect, for every reconciliation outcome including conflict.
- **SC-004**: A reconnection where the host has not changed any edited file completes with
  **zero** developer interactions.
- **SC-005**: A file changed offline and on the host in non-overlapping regions reconciles with
  **zero** developer interactions, and both changes are present afterwards.
- **SC-006**: **100%** of genuinely overlapping changes prompt; **zero** are resolved by the
  client choosing a side.
- **SC-006a**: **100%** of offline edits to files the client cannot merge prompt on
  reconnection, including those the host did not change.
- **SC-002a**: Across **10** files each saved offline at least **3** times before reconnecting,
  the base used for reconciliation is in **100%** of cases the content the host last confirmed —
  never an intermediate offline version.
- **SC-006b**: For a corpus of at least **20** file pairs edited on both sides, the client's
  decision to merge or prompt matches a standard version-control three-way merge in **100%** of
  cases.
- **SC-007**: A file previously opened while online opens offline in under **200 ms**.
- **SC-008**: Offline path search over a workspace of **50,000** cached paths returns in under
  **1 second**.
- **SC-009**: Interactive actions taken while prefetch is running are no slower than the same
  actions with prefetch idle, within **10%**.
- **SC-010**: Once prefetch reports that it has completed or stopped at the cache budget,
  **100%** of the project's manifest files and the files changed in the bounded set of recent
  commits are readable offline. Stated against prefetch's own report rather than against a
  wall-clock session, because the criterion is about what prefetch achieves and a timed version
  would need a five-minute wait in a suite that runs in ninety seconds — which is a test that
  gets skipped, and a skipped test reads as coverage.
- **SC-010a**: **Zero** cached files are evicted by prefetch.
- **SC-011**: Reconciling **100** files with pending edits completes in under **10 seconds**.
- **SC-012**: **Zero** requests are issued to read a cached file while offline.

## Design deviations

Two surfaces this feature needs do not exist in the signed-off prototype. Recorded here rather
than decided silently, and both built from design-system tokens only, so a designer can move
either without unpicking an improvised value (Principle I). The same shape as F011's branch
indicator, which the design system absorbed without incident.

- **The conflict interface.** FR-021 requires the client to prompt, and a prompt needs somewhere
  to happen. Three versions and a choice, in `client/ui/lib/offline/ConflictPanel.svelte`.
- **The path-search surface.** FR-007 requires that the developer can run a path search and that
  the interface not present the results as complete, and there was no search surface in the client
  at all — a filter input, a results list and a "showing cached results" caveat, in
  `client/ui/lib/workspace/PathSearch.svelte`. Added during implementation once it became clear
  that FR-007 named a surface rather than a capability; the reviewer chose to build it here rather
  than defer it to F013 or narrow the requirement.

This section was missing until implementation. plan.md's Principle I row said the deviation was
"recorded here and in spec.md" and spec.md recorded nothing — the same shape as the Principle VII
row that analysis pass 18 corrected, which cited evidence in a document that did not carry it.
Thirty-one analysis passes did not catch either, because a claim about another artifact reads as
true until somebody opens that artifact.

## Assumptions

- **The developer performs version control operations elsewhere.** This feature reconciles file
  content; it does not commit, pull, or push. F011 reads git state and F010 gives them a
  terminal.
- **Offline is a state of the connection, not of the workspace.** A local workspace has no remote
  and is never offline in this sense (§13.1).
- **The base for reconciliation is the last content confirmed with the host**, not the last
  content the developer saw. These differ when a file was fetched, edited offline, and the
  application relaunched; the stored hash is what matters.
- **A line-oriented merge is the right shape** for the files in scope, which are source files.
  Files the client cannot merge that way stay editable and always prompt, so the merge engine
  never meets a case it cannot handle (FR-017a, FR-025a).
- **The conflict interface is part of this feature.** A-OFFLINE requires conflicts to prompt, and
  a prompt with nowhere to happen is not a requirement that can be met.
- **Recent commits and manifests are a good prefetch heuristic**, taken from §11.4. Whether it is
  the *best* heuristic is a question for measurement after this ships, not a blocker for it.
- **Reconciliation happens once per reconnection**, not continuously. A file edited offline and
  then edited again after reconnecting is an ordinary online edit.
