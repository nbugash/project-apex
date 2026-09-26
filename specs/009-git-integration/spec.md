# Feature Specification: Git Integration

**Feature Branch**: `feature/F011-git-integration`

**Created**: 2026-09-26

**Status**: Draft

**Input**: Feature map entry F011 `git-integration` — engine-side porcelain v2 parsing and watch
triggers, `git_status` projection with transactional merge, file tree colouring from cached
status, diff gutter coordinates via `git/getFileDiff`, bulk invalidation on branch switch, and
the current branch in the status bar.

## On the source of values

Nothing here invents a number, a colour or a method name. The protocol is §4.8's *Git* table,
the division of labour is §12.1, the pipeline is §12.2, the presentation is §12.3 and the branch
switch is §12.4. That git status is **not** a cache signal is §5.3, stated there before this
feature existed. The two watches inside `.git/` are A-GITWATCH, recorded before this
specification was written because they amend A-IGNORE, which binds F004 and F013 as well.

Where this document fixes something the system specification leaves open, it says so in
*Clarifications* and gives the reasoning.

## What this feature is not

- **Not git operations.** No commit, stage, unstage, push, pull, branch, merge or revert. This
  feature *reads* git and shows what it read. A developer stages files in their terminal — which
  F010 gives them — and the tree follows.
- **Not a diff viewer.** Gutters mark which lines changed. Showing the before-and-after text
  side by side is a surface this feature does not build.
- **Not history.** No log, no blame, no file-at-revision.
- **Not conflict resolution.** A conflicted file is reported as conflicted. Resolving it is the
  developer's business, in the editor or the terminal.
- **Not a cache signal.** §5.3 settles this: git status colours the tree and never invalidates
  content. The file a developer just saved is `MODIFIED`, and invalidating on that signal would
  discard cached content for precisely the files in use.

## Clarifications

### Session 2026-09-26

- Q: How does the engine learn that git status has changed? → A: Two targeted watches on the
  repository's `HEAD` and `index`, recorded as A-GITWATCH. Refreshing from the workspace's own
  file events was rejected because an index-only change — `git add`, `git reset`, `git stash` —
  alters no working-tree file, so staged and unstaged state would stay stale until some
  unrelated file happened to change.
- Q: Does this feature show the current branch in the status bar? → A: Yes. §12.3 requires it and
  the branch already arrives in both `git/getStatus` and `git/onStatusUpdate`; without it the
  branch would be parsed, transmitted and cached with nothing displaying it. Added to the feature
  map as a sixth subfeature rather than decided quietly during implementation.
- Q: Where are `HEAD` and `index` when `.git` is not a directory? → A: In a linked worktree or a
  submodule, `.git` is a **file** containing a `gitdir:` pointer, so `<root>/.git/HEAD` does not
  exist. The engine resolves the repository's actual git directory and watches `HEAD` and `index`
  there. Assuming a directory would leave every worktree and submodule silently without status —
  working, never updating, with nothing to indicate why.
- Q: A path can be staged **and** modified again; §4.8 carries one status per path. Which? → A:
  The unstaged state wins, and `CONFLICT` overrides everything. A staged file with further edits
  still holds work that is recorded nowhere, and `MODIFIED` says so; reporting `STAGED` would
  tell the developer their work is safe when part of it is not. Revisited in clarify.

## Design deviations

Principle I makes the signed-off prototype the authority on what this application looks like.

| Surface | In the prototype? | Consequence |
|---|---|---|
| File tree VCS marker | **Yes.** The column exists in the prototype, `ds-sync` extracted `--vk-tree-vcs-size` from it, and F000 reserved the slot in the tree row so filling it would not reflow every row | No deviation. This feature fills a column the design already has |
| Current branch in the status bar | **No.** The prototype's status bar has no branch indicator; searching its visible text for "branch" returns nothing | A deviation, taken deliberately: §12.3 requires it. Built from design-system tokens only, with no new values, so a designer can move or restyle it without unpicking an improvised colour |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See which files I have changed (Priority: P1)

A developer edits three files across a repository of several thousand. Without leaving the
editor, the file tree shows which three, and whether each is modified, newly created or staged.
They did not run a command and did not wait.

**Why this priority**: This is the feature. Everything else here refines or extends it, and it is
the only part that changes what a developer knows at a glance about work in progress. It is also
the slice that proves the whole pipeline — the engine noticing, parsing and reporting, and the
client storing and drawing.

**Independent Test**: Open a workspace that is a repository, change a file outside the client,
and confirm the tree marks it. Delivers value alone: a developer can see their changes.

**Acceptance Scenarios**:

1. **Given** a clean repository, **When** a file is modified on the host, **Then** the tree marks
   that file as modified and marks no other file.
2. **Given** a modified file, **When** it is staged on the host, **Then** the tree's mark for it
   changes to reflect staging, without the developer refreshing anything.
3. **Given** a file that git does not track, **When** the tree renders, **Then** it is marked as
   untracked and is distinguishable from a modified file without relying on colour.
4. **Given** a repository with changes, **When** a folder of files is rendered, **Then** no
   request is issued per file.
5. **Given** a modified and saved file, **When** its status arrives, **Then** the number of files
   held in the cache is unchanged.
6. **Given** a workspace whose files are marked, **When** the client is restarted, **Then** the
   same files are marked as soon as the tree renders, without waiting for a refresh (FR-013).

---

### User Story 2 - Know which branch I am on (Priority: P2)

A developer glances at the status bar and sees the branch they are working on, so that a change
made on the wrong branch is noticed before it is committed rather than afterwards.

**Why this priority**: Small, valuable, and independent of the tree. Second rather than first
because a developer who cannot see *what* changed is worse off than one who cannot see *where*.

**Independent Test**: Open a repository and confirm the status bar names its branch; switch
branches on the host and confirm it follows.

**Acceptance Scenarios**:

1. **Given** a repository on `main`, **When** the workspace opens, **Then** the status bar shows
   `main`.
2. **Given** a workspace showing a branch, **When** the branch is switched on the host, **Then**
   the status bar shows the new branch.
3. **Given** a workspace that is not a repository, **When** it opens, **Then** the status bar
   shows no branch and no error.
4. **Given** a repository with a detached HEAD, **When** the workspace opens, **Then** the status
   bar identifies the commit rather than showing an empty branch.

---

### User Story 3 - See which lines I changed (Priority: P2)

A developer opens a file they have modified and sees, in the editor's gutter, which lines they
added, which they changed and where they deleted something — without the editor fetching the
file's previous contents.

**Why this priority**: Valuable and self-contained, and it depends on an open editor, which makes
it a later slice than the tree. It is also the one part that touches the interaction budget,
because it happens on opening a file.

**Independent Test**: Open a modified file and confirm the gutter marks the changed lines; open
an unmodified file and confirm it marks none.

**Acceptance Scenarios**:

1. **Given** a file with added, changed and deleted lines, **When** it is opened, **Then** the
   gutter marks each kind distinguishably.
2. **Given** an unmodified file, **When** it is opened, **Then** the gutter marks nothing.
3. **Given** an open file with gutter marks, **When** its git state changes on the host, **Then**
   the marks follow.
4. **Given** a file being diffed, **When** the diff is obtained, **Then** no file content is
   transmitted for the purpose of drawing it.

---

### User Story 4 - Switch branches without the client falling behind (Priority: P3)

A developer switches branches on the host, changing thousands of files. The client does not
freeze, does not download the new branch, and does not go on showing the old branch's state.

**Why this priority**: It protects the other three rather than adding a surface of its own, and
its failure mode — a client quietly showing a branch the developer left — is the kind that is
discovered late.

**Independent Test**: Switch a branch that changes many files and confirm the client receives one
bulk invalidation rather than one event per file, and that stale git state is gone.

**Acceptance Scenarios**:

1. **Given** a workspace on a branch, **When** the branch is switched on the host, **Then** the
   client receives a single bulk invalidation rather than one event per changed file.
2. **Given** a bulk invalidation, **When** it is applied, **Then** the tree is not eagerly
   refetched.
3. **Given** a workspace that has switched branches, **When** the tree renders, **Then** it shows
   no file marked from the previous branch's status.

---

### Edge Cases

- **The workspace is not a git repository.** Common, and not an error: the tree renders
  uncoloured, the status bar shows no branch, and nothing is reported as having failed.
- **`git` is not installed, or is too old to be usable, on the host.** Degrades exactly as
  "not a repository" does, rather than failing the workspace.
- **`.git` is a file rather than a directory** — a linked worktree or a submodule. The
  repository's real git directory is resolved and watched there.
- **A repository with no commits yet.** `HEAD` names a branch that does not exist. The branch is
  shown; every tracked file is reported as it is, with nothing crashing on the missing commit.
- **Detached HEAD.** No branch name exists to show; the commit identifies the position instead.
- **A path is staged and then modified again.** One status per path, and the unstaged state wins.
- **A file is renamed.** Reported under the path it now has.
- **A conflicted file.** Reported as conflicted, which overrides every other state.
- **Status arrives for a workspace that has since closed.** Discarded rather than applied to
  whatever workspace now holds that identity.
- **Two status updates arrive close together.** The later one wins, and the projection never
  shows a mixture of the two.
- **An enormous status.** A repository where most files differ must not make the tree unusable.
- **The connection drops.** The last known git state remains visible on the same terms as other
  cached content, rather than silently clearing and implying everything is unchanged.

## Requirements *(mandatory)*

### Functional Requirements

**Determining status**

- **FR-001**: Git status MUST be computed on the engine, never on the client (§12.1).
- **FR-002**: The engine MUST refresh status when the repository's `HEAD` changes and when its
  `index` changes (A-GITWATCH).
- **FR-003**: A change that alters the index but no working-tree file — staging, unstaging,
  stashing — MUST refresh status.
- **FR-004**: The engine MUST resolve the repository's actual git directory rather than assuming
  `<root>/.git` is a directory, so that linked worktrees and submodules are watched correctly.
- **FR-005**: The two watched paths MUST NOT be reported as workspace file events (A-GITWATCH).
- **FR-006**: The engine MUST report the current branch and the set of changed paths, both when
  status changes and on demand for a client that has just connected (§4.8).
- **FR-007**: Each reported path MUST carry exactly one state, drawn from the set §4.8 defines.
- **FR-008**: Where a path has both a staged and an unstaged state, the unstaged state MUST be
  the one reported; a conflicted state MUST override every other.

**Holding it on the client**

- **FR-009**: A status update MUST be applied as a single transaction: the workspace's previous
  status is replaced by the new set, with no observable state in which both or neither is present.
- **FR-010**: Applying a status update MUST NOT alter which files are cached, nor any cached
  content (§5.3).
- **FR-011**: A status update for one workspace MUST NOT alter another workspace's status.
- **FR-012**: A status update naming a workspace the client does not have open MUST be discarded.
- **FR-013**: Git status MUST survive a restart, so that a reopened workspace shows what it
  showed before without waiting for a refresh.

**Showing it**

- **FR-014**: The file tree MUST show each file's git state.
- **FR-015**: Git states MUST be distinguishable from one another without relying on colour.
- **FR-016**: A file with no git state MUST render exactly as it does today.
- **FR-017**: Rendering the tree MUST NOT issue a request per file; it reads what is already held.
- **FR-018**: The status bar MUST show the current branch.
- **FR-019**: Where there is no branch to show — not a repository, or a detached HEAD — the
  status bar MUST NOT show an empty or placeholder branch.

**Lines**

- **FR-020**: Opening a file with changes MUST show which lines were added, changed and deleted.
- **FR-021**: Diff information MUST be line coordinates only. File contents MUST NOT be
  transmitted for the purpose of drawing a gutter (§12.3).
- **FR-022**: A file with no changes MUST show no gutter marks.
- **FR-023**: Gutter marks for an open file MUST follow a change to that file's git state.

**Branch switches**

- **FR-024**: A branch switch MUST produce a single bulk invalidation rather than one event per
  changed file (§12.4).
- **FR-025**: A bulk invalidation MUST NOT cause the client to eagerly refetch the tree.
- **FR-026**: After a branch switch, no file MUST remain marked from the previous branch's status.

**When git is not there**

- **FR-027**: A workspace that is not a git repository MUST be fully usable, showing no git state
  and reporting no failure.
- **FR-028**: A host with no usable `git` MUST degrade exactly as a non-repository does.

**When the engine is not there**

- **FR-029**: With no connection, the last known git state MUST remain visible rather than
  clearing, and MUST be presented on the same terms as other content that cannot currently be
  confirmed.

### Key Entities

- **Git status entry**: one changed path and its single state, belonging to one workspace. The
  set of entries for a workspace is replaced wholesale, never merged row by row.
- **Branch**: what the repository currently has checked out, or the absence of one.
- **File diff**: for one file, the line coordinates that were added, deleted and modified. Holds
  no file content.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A file changed on the host is marked in the tree within **2 seconds**, with no
  action by the developer.
- **SC-002**: A file staged on the host — a change to the index and to no working-tree file — is
  reflected in the tree within **2 seconds**.
- **SC-003**: Rendering a folder of any size with git state issues **zero** requests.
- **SC-004**: Opening a modified file shows its gutter marks within **250 ms**, measured as a p99
  over at least 100 samples with the measured value printed (§1.4, A-NFR).
- **SC-005**: A branch switch that changes 10,000 files produces **exactly one** invalidation.
- **SC-006**: Applying any number of status updates changes the count of cached files by **zero**.
- **SC-007**: A workspace that is not a repository shows no git state, shows no branch, and
  surfaces **zero** errors.
- **SC-008**: Every git state a file can be in is distinguishable from every other in greyscale.
- **SC-009**: The branch shown matches the repository's actual branch in **100%** of checks,
  including immediately after a switch.
- **SC-010**: No file content is transmitted for diff purposes — measured as **zero** bytes of
  file content in diff responses.
- **SC-011**: After a branch switch, **zero** files remain marked from the previous branch.

## Assumptions

- **The developer performs git operations elsewhere.** F010 gives them a terminal on the same
  host; this feature reads the result. No requirement here implies a button that changes the
  repository.
- **One repository per workspace root.** Nested repositories below the root are not enumerated;
  a submodule is watched only when it is itself the workspace root.
- **The status bar is the right home for the branch**, on the strength of §12.3, even though the
  prototype has no such indicator. Recorded as a design deviation above.
- **`git` on the host is current enough to report status in a machine-readable form.** Where it
  is not, FR-028 applies and the workspace degrades as a non-repository. The precise mechanism is
  a planning decision, not a requirement.
- **The cache schema already holds git status.** F003 created the table and index deliberately
  for this feature, so no migration is required — which is why no requirement here mentions one.
- **Two seconds is the right freshness bound** for SC-001 and SC-002. Nobody waits on a git
  status the way they wait on a keystroke; the bound exists so "eventually" cannot pass as a
  result, not because two seconds is perceptible.
