# Data Model: Git Integration

Phase 1. Entities, their identity, and the rules that hold about them. Field types are named
where they carry a constraint; where they do not, the shape is left to `design.md`.

---

## GitState

The state of one path in one workspace, as git last reported it.

| Field | Meaning |
|---|---|
| `workspace_id` | Which workspace. Part of the identity. |
| `relative_path` | Which path, workspace-relative. Part of the identity. |
| `status` | Exactly one of the five states below. |

**Identity is (`workspace_id`, `relative_path`)** — deliberately *not* the file tree's identity
for a file. An untracked file in a folder nobody has expanded has no tree entry, and a deleted
file's tree entry is removed by the indexer; both must still carry state, and keying on the tree
would drop exactly those two. See spec.md, *Clarifications*.

**The five states**, fixed by §4.8: `MODIFIED`, `UNTRACKED`, `STAGED`, `DELETED`, `CONFLICT`.

**Derivation rule.** git reports staged and unstaged separately; one state is stored. A conflict
wins outright; otherwise the unstaged state wins, and the staged state is used only when there is
no unstaged change. The reasoning is in spec.md; the verified character forms are in research.md.

**Lifecycle.** A `GitState` row is never edited. The set for a workspace is replaced wholesale by
an update, and rows exist only between one update and the next.

**Not stored**: staged and unstaged state separately, rename origins, file modes, object ids.
None of them is displayed, and §4.8 does not carry them.

---

## StatusUpdate

One complete answer from the engine about a workspace, possibly delivered across several pages.

| Field | Meaning |
|---|---|
| `workspace_id` | Which workspace this describes. |
| `branch` | The current branch, or its absence. See *BranchPosition*. |
| `changes` | The `GitState` entries in this page. |
| `next_cursor` | Present exactly when further pages remain. |

**The update is the whole sequence, not the message.** An update that sets `next_cursor` is
incomplete, and applying it as though it were complete would mark every path beyond the first
page as unchanged. The replacement commits when the page without a cursor arrives (A-GITPAGE).

**Validation.**

- An update naming a workspace the client does not hold is discarded, not stored (FR-012).
- Pages of one update must agree on the workspace; a page that disagrees ends the accumulation
  rather than being merged.
- A partially accumulated update whose remaining pages never arrive is discarded, leaving the
  previously applied state intact (FR-009a).

**State transitions.**

```text
idle ──update arrives──▶ accumulating ──final page──▶ committed ──▶ idle
                              │
                              └──pull fails / workspace closes──▶ discarded ──▶ idle
```

---

## BranchPosition

Where the repository is, which is not always a branch.

| Case | Carries | Shown as |
|---|---|---|
| On a branch | the branch name | the name |
| Detached | the commit id | the commit, short form |
| Not a repository, or git unavailable | nothing | no branch indicator at all |

Modelled as three cases rather than a nullable string because "no branch" and "a branch named
nothing" are different facts, and git reports the detached case as the literal `(detached)` in
the name's position — a value that reads as a branch name unless it is given its own case
(research.md).

---

## FileDiff

Which lines of one file differ from what git has recorded.

| Field | Meaning |
|---|---|
| `relative_path` | Which file. |
| `added` | Line ranges present in the working copy and not before. |
| `deleted` | Positions where lines were removed. |
| `modified` | Line ranges that changed in place. |

**Coordinates only.** No file content, ever — the client already holds the text (§12.3, FR-021).
A deletion has no lines in the new file, so it is a position rather than a range; a parser that
modelled every entry as a non-empty range would have nowhere to put it.

**Not persisted.** Diffs are fetched when a file is opened and discarded when it closes. They
describe a moment and are cheap to ask for again; storing them would create a second thing that
can be stale about the same file.

---

## Relationships

```text
Workspace ─1──many─▶ GitState        (replaced wholesale per update)
Workspace ─1───1───▶ BranchPosition  (replaced per update)
File      ─1───0/1─▶ FileDiff        (transient, only while open)
```

`GitState` references a workspace and a path. It deliberately holds **no** foreign key into the
file tree: the tree is a projection of what has been listed, and git state exists for paths that
have not been.

## What this model does not touch

Cache validity. §5.3 settles it and FR-010 repeats it: a file's git state has no bearing on
whether its cached content is valid. The two live in the same database and share nothing else —
a file that is `MODIFIED` has cached content that is exactly as valid as it was before, because
the developer's own save is what made it modified.
