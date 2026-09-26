# Quickstart: Git Integration

How to prove this feature works, and how to prove the proofs are real. Phase 1.

---

## 1. Prerequisites

- A Linux host with `git` 2.30 or later. The probe behind [research.md](./research.md) used
  2.43.0; `--porcelain=v2` has been stable since 2.11.
- The workspace repository, built: `cargo build --workspace && npm ci && npm run build`.
- A scratch repository for the end-to-end suite. It is created by the specs themselves, under
  `.e2e-workspace`, and is never the checkout — a test that writes into the repository it is
  testing can destroy work.

---

## 2. The whole gate

```bash
make gate
```

Runs the workspace suite, both end-to-end runs, the fidelity gate and the no-network check. The
editor and git specs are in the **live** run, because git status needs a real engine and a real
repository; `terminal-live.spec.ts` runs last, for the reason `wdio.conf.ts` records.

---

## 3. The measurements (§8 of this guide)

Numbers, not verdicts. Each is printed by the command beside it.

| Criterion | Command | Bound |
|---|---|---|
| **SC-001** time from a host change to the tree marking it | `npm run e2e:live -- --spec tests/e2e/git-status.spec.ts` | < 2 s |
| **SC-002** time from a host `git add` to the tree reflecting it | same spec | < 2 s |
| **SC-003** requests issued rendering a folder with git state | same spec | **0** |
| **SC-004** open-to-gutter, p99 over ≥100 samples | `cargo test -p apex-shell --test git_gutter_budget -- --nocapture` | < 250 ms |
| **SC-005** invalidations for a 10,000-file branch switch | `cargo test -p apex-engine --test git_branch_switch` | **1** |
| **SC-012** paths marked when 5,000 change | `cargo test -p apex-engine --test git_status_paging` | **5000** |
| **SC-013** status reports from a 50-change burst | `cargo test -p apex-engine --test git_coalesce` | **1** |

Record each in the *Validation record* below when the feature is implemented.

---

## 4. The negative checks

Each of these must be able to **fail**. The condition beside it is what makes that possible; a
check whose condition is absent passes for any implementation, which is the failure mode this
table exists to catch.

| Must not happen | The condition that lets it fail |
|---|---|
| A paged status is applied from its first page (A-GITPAGE) | The fixture repository genuinely has more than one page of changes — more than 1000 — so a first-page-only implementation visibly marks the rest clean |
| An index-only change goes unnoticed (FR-003) | The test really runs `git add` and really touches no working-tree file, so a workspace-watcher-driven implementation sees nothing |
| A worktree is silently never updated (FR-004) | The fixture is a **real linked worktree**, where `.git` is a file, so an implementation assuming `<root>/.git/HEAD` finds nothing to watch |
| Git status invalidates the cache (FR-010, §5.3) | Files are genuinely cached before the status arrives, and the count is read afterwards — not merely asserted about the code |
| A burst becomes N computations (FR-006b) | The burst is real and rapid enough to span the trailing edge, and the count measured is of status *computations*, not of notifications |
| File content reaches the client in a diff (FR-021) | The file genuinely has content that would show up, and the assertion inspects the payload rather than the parser |
| A rename desynchronises the parser (research.md) | The fixture contains a real rename, whose original path is a separate NUL field |
| Detached HEAD shows "(detached)" as a branch name | The fixture really detaches HEAD, so an implementation treating the header as a name displays the literal string |

---

## 5. The mutation checks

Break the property, confirm the test fails, restore. A test that passes both ways is not a test.

1. **Apply a status update from its first page, ignoring the cursor.** The paging test must fail
   on the count of marked files — not on an error.
2. **Key git state by the tree's file identity instead of by path.** The untracked-in-unexpanded-
   folder test must fail.
3. **Take the index character in preference to the worktree character.** The staged-and-modified
   test must fail.
4. **Assume `<root>/.git` is a directory.** The worktree test must fail, and only that one.
5. **Remove the trailing edge, refreshing on every index write.** The coalescing count must fail.
6. **Emit the git watch's events as workspace file events.** A file-event test must fail — this
   is the leak A-GITWATCH's separate service exists to make impossible, so if nothing fails, the
   separation is not doing the work claimed for it.
7. **Return diff hunks with their context lines included.** The zero-content assertion must fail.
8. **Treat `# branch.head (detached)` as a branch name.** The detached test must fail.

Mutations 2 and 4 are worth attention: each is a plausible simplification that leaves every other
test green.

---

## 6. Validation record

*(Filled in when the feature is implemented. Numbers, not verdicts.)*
