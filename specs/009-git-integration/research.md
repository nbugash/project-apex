# Research: Git Integration

Phase 0. Every finding below was checked against real git 2.43.0 in a scratch repository rather
than recalled, because most of what follows is a detail of an output format and a recalled
format is a guess with good grammar. The probe covered a modified file, a deleted file, an
untracked file, a rename, a merge conflict, a detached HEAD and a linked worktree.

---

## Asking git for status

**Decision.** `git status --porcelain=v2 -z --branch`, run on the host by the engine.

**Rationale.** `--porcelain=v2` is git's documented machine-readable format with a stability
guarantee, and unlike v1 it reports the staged and unstaged state as two separate characters,
which is what makes the precedence rule in spec.md implementable at all. `-z` terminates fields
with NUL, so a path containing a newline, a quote or a space needs no unquoting — and git's
default output *does* quote such paths, which is a decoding step this avoids entirely. `--branch`
adds the header lines that carry the branch, so status and branch are one invocation rather than
two.

**Verified output** for a modified file, a deleted file and an untracked file:

```text
# branch.oid 9581378fedf01df771e8de90617d75c09bb41b4c
# branch.head master
1 .M N... 100644 100644 100644 f384549c... f384549c... src/a.txt
1 .D N... 100644 100644 000000 587be6b4... 587be6b4... tracked.txt
? untracked.txt
```

**Alternatives considered.** `--porcelain=v1`: one status character per path, collapsing staged
and unstaged before we can apply our own precedence, and its path quoting has to be undone.
`git diff --name-status` plus `git ls-files --others`: two invocations, two formats, and no
conflict reporting. Reading `.git/index` directly: no subprocess and a hard dependency on a file
format git does not promise to keep.

---

## Parsing `-z` output, and the trap in it

**Decision.** Parse by record type — `1`, `2`, `u`, `?`, `!` and `#` — and let the record type
decide how many NUL-terminated fields to consume.

**Rationale.** This is not a "split on NUL and take one record per field" format, and treating it
as one is wrong in a way that only shows up on a rename. A `2` record — a rename or copy — is
followed by **its original path as a separate NUL-terminated field**. A naive splitter reads that
old path as the start of a new record and produces a phantom entry, then desynchronises for
everything after it.

Verified: `2 R. N... 100644 100644 100644 587be6b4... 587be6b4... R100 renamed.txt` followed by a
NUL and then the original path.

**Alternatives considered.** Splitting on NUL and heuristically detecting record starts by the
first character: works until a file is named `1 something`, which is legal. Using `--porcelain=v2`
without `-z` and unquoting: swaps a known parsing rule for an unknown one.

---

## One state per path, from two characters

**Decision.** `CONFLICT` for any `u` record. Otherwise, where the worktree character is not `.`,
that character decides the state; where it is `.`, the index character gives `STAGED`. `?`
records are `UNTRACKED`.

**Rationale.** §4.8 carries one status per path, so two characters must become one. The unstaged
character winning is spec.md's confirmed clarification: a staged file with further edits still
holds work recorded nowhere, and reporting it as staged says the opposite. `u` overrides
everything because a conflict is the only state that blocks the developer from proceeding.

Verified: `.M` modified-in-worktree, `.D` deleted-in-worktree, `R.` renamed-and-staged,
`u AA` both-added conflict.

**Alternatives considered.** Reporting the index character in preference: tells a developer their
work is safe when part of it is not. Adding a combined state: a protocol change for a
presentation problem.

---

## Noticing that status changed

**Decision.** Two watches — `HEAD` and `index` — inside the directory `git rev-parse --git-dir`
reports, held by a git watch service that is **separate** from the workspace watcher.

**Rationale.** A-GITWATCH settles why these two paths and not the workspace's own events: an
index-only change alters no working-tree file, so `git add`, `git reset` and `git stash` are
invisible to the file watcher.

Resolving the git directory rather than assuming `<root>/.git` is what makes worktrees and
submodules work, and this was checked rather than assumed. In a linked worktree, `.git` is an
**80-byte file**, and `rev-parse --git-dir` resolves to
`<main>/.git/worktrees/<name>/`, whose contents are `HEAD ORIG_HEAD commondir gitdir index logs`
— both watched files present, in a directory that is not `<root>/.git` and is not even under the
worktree.

A separate service rather than an exception inside the existing watcher's exclusion filter: it
makes "these events never become `workspace/onFileEvent`" a property of there being no code path
between them, rather than of a filter staying correct while two features change around it. It
costs one more inotify instance per workspace.

**Alternatives considered.** An allowlist that overrides the exclusion set for two paths: fewer
moving parts, and it puts git events inside the machinery whose entire job is producing file
events, one missed branch away from leaking. Polling: rejected in clarify.

---

## Keeping a burst from becoming N computations

**Decision.** A 100 ms trailing edge on index changes, plus at most one status computation in
flight with at most one more scheduled.

**Rationale.** The trailing edge is A-COALESCE's existing figure for repeated changes to one path,
and the index is one path. But a trailing edge alone is not enough: a rebase writes the index
repeatedly over *seconds*, so every 100 ms gap starts another full-repository status. Holding one
run in flight and collapsing everything that arrives during it into a single follow-up bounds a
burst of any length at two computations — the one already running, and one more that reflects
everything that happened while it ran.

**Alternatives considered.** A longer trailing edge: makes the 2 s freshness bound a function of
the window rather than of the work, and still admits repeat runs across a long rebase. A queue of
pending runs: N computations with extra steps.

---

## Paging a status git produces all at once

**Decision.** The engine computes the whole status, then serves it in pages of at most 1000
entries; `git/onStatusUpdate` carries the first page and a cursor when more remain.

**Rationale.** A-GITPAGE settles why paging rather than a cap. What research adds is that git
does not stream usefully here: status is computed and emitted as one run, so the pages come from
slicing a result the engine already holds, not from resuming git. The cursor is therefore a
position in a completed snapshot, which is what makes a later page consistent with the first —
a cursor that resumed git could return pages from two different repository states.

The snapshot is held only while a pull is in progress and is discarded when the last page is
served or the client goes away.

**Alternatives considered.** Re-running git per page: cheaper in memory and capable of returning
a self-contradictory picture. Streaming git's stdout into pages as it arrives: avoids holding the
result, and makes a page boundary depend on process scheduling.

---

## Diff coordinates without file contents

**Decision.** `git diff --unified=0 -- <path>`, parsing only the `@@` hunk headers.

**Rationale.** §12.3 requires line coordinates and forbids transmitting contents. `--unified=0`
removes context lines, so the output is hunk headers plus the changed lines themselves; taking
only the headers gives exactly the coordinates and discards the text without ever sending it.

Verified: a one-line change and a one-line insertion produced `@@ -2 +2 @@` and `@@ -4,0 +5 @@`.
The second is the form that matters — `-4,0` means zero old lines at old position 4, which is an
insertion, and `+5` is where it lands in the new file. A parser that assumed `-<n>,<m>` always has
both numbers would mishandle `@@ -2 +2 @@`, where the counts are elided when they are 1.

**Alternatives considered.** `--numstat`: counts only, no positions, so no gutter. Computing the
diff on the client from cached content: the client may not hold the committed version at all, and
fetching it would transmit contents §12.3 rules out.

---

## The branch, and when there is not one

**Decision.** Take the branch from the `# branch.head` header. The literal value `(detached)`
means there is no branch, and the commit from `# branch.oid` identifies the position instead.

**Rationale.** Verified: on a detached HEAD, git reports `# branch.head (detached)` — a literal
string in the name's position, not an empty field. Code that treated the header as a name would
put the word "(detached)" in the status bar as though it were a branch, which reads as a
peculiarly named branch rather than as the absence of one.

An unborn branch — a repository with no commits — reports the branch name normally with
`# branch.oid (initial)`, so the branch shows and the commit does not.

**Alternatives considered.** `git symbolic-ref HEAD`: a second invocation, and it fails on a
detached HEAD rather than reporting it, so the absence has to be inferred from an error.

---

## Keying git state on the client

**Decision.** (workspace, path), replacing the existing key.

**Rationale.** spec.md's clarification settles the what; research confirms the cost is a schema
edit rather than a data migration, because nothing has ever written a row to that table. The
table and its lookup index were created by F003 in anticipation of this feature, which is why
there is a table to re-key at all.

**Alternatives considered.** Both rejected in clarify; recorded there.

---

## Colouring without inventing colours

**Decision.** Five states, five design-system tokens, read from the mounted element at runtime —
the pattern A-EDITPALETTE established for the editor's syntax colours.

**Rationale.** The tree's VCS marker column is in the signed-off prototype and F000 reserved the
slot, so this feature fills a column the design already has. Reading tokens off the element keeps
the stylesheet the one place a colour is decided, and `lint:ds` refuses a literal — which is how
F006's first attempt at a new surface was caught inventing token names.

Legibility without colour (FR-015) is a separate obligation the marker must carry in its glyph,
not only in its hue, and is asserted on luminance rather than on colour, following
`rail-greyscale.spec.ts`.

**Alternatives considered.** Colour alone: fails FR-015 and the accessibility gate. New tokens
for git states: this application deciding what the design system looks like.

---

## When there is no git, or no repository

**Decision.** Both degrade identically: no status, no branch, no error surfaced, workspace fully
usable.

**Rationale.** A workspace that is not a repository is an ordinary case, not a failure, and a
host without git is a configuration the developer can fix but which must not cost them the
workspace. Treating either as an error would put a permanent failure indicator on a client that
is working correctly.

The distinction is kept in the engine's reply — "this is not a repository" and "git could not be
run" are different facts — but both produce the same empty status, so the client has one path.

**Alternatives considered.** Reporting git's absence as a workspace-level failure: makes a
non-git workspace unusable for a feature it does not use. Silently retrying: hides a fixable
configuration behind an indefinite absence of colour.
