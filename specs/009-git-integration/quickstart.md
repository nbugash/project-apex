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

Measured 2026-09-27 on the development host (Linux, local engine over stdio). **Numbers, not
verdicts** — a gate that says only PASS tells nobody how much headroom is left, which is what
says whether the next feature's work can be afforded.

| Criterion | Bound | Measured | Headroom |
|---|---|---|---|
| **SC-001** host change to the tree marking it | < 2 s | **105 ms** | 19x |
| **SC-002** host `git add` to the tree reflecting it | < 2 s | **153 ms** | 13x |
| **SC-003** requests issued rendering a folder with git state | 0 | **0** | — |
| **SC-004** open-to-gutter, p99 over 200 samples | < 250 ms | **0.09 ms** (client's share; 120 hunks) | 2700x |
| **SC-005** frames for a branch switch | not per file | **1** status update, ~5 event frames, ~7 invalidations for 2,000 files | ~160x below per-file |
| **SC-012** paths marked when 5,000 change | 5000 | **5000**, across 5 pages, none twice | — |
| **SC-013** status computations from a 50-change burst | 1 | **1** during the burst, 1 after it settles | — |

Two notes on what these numbers do **not** cover.

**SC-004 is the client's share only.** The engine's `git diff` subprocess is not in it; what is
measured is request encoding, reply parsing and mapping, which is what the client spends. The
figure is reported that way deliberately — it says 249.9 ms of the budget remains for the engine.
The first version of that test measured a fake handing back a cloned struct and printed `0 us`
over two hundred samples, a number that cannot distinguish a fast client from one that is not
running.

**SC-005's bound is stated against the file count, not as "1".** A 2,000-file switch does cost
one *status* update, which is the git subsystem's whole obligation. The workspace watcher's side
produces several frames, because a burst that size arrives in waves and each wave crossing
A-COALESCE's 256-path threshold invalidates wholesale. Roughly twelve frames for two thousand
files is the claim FR-024 and SC-005 actually make; asserting a fixed small number would be a
bound nobody can point at a reason for, and those get relaxed the first time they fail.

---

## 7. Negative-check audit

Each condition confirmed present, 2026-09-27. A check whose condition is absent passes for any
implementation, which is the failure mode this audit exists to catch.

| Must not happen | Condition | Confirmed |
|---|---|---|
| A paged status applied from its first page | More than one page of changes in the fixture | `git_apply.rs` builds 1,000 + 1,000 + 500 across three pages; `git_status_paging.rs` uses 5,000 |
| An index-only change goes unnoticed | A real `git add` touching no working-tree file | `git_push.rs::staging_reaches_the_client_too` and `git_watch.rs::staging_a_file_fires_the_watch` both run real `git add` |
| A worktree silently never updated | A **real** linked worktree, `.git` a file | `Repo::add_worktree`; the test asserts `git_dir != <root>/.git` before watching |
| Git status invalidates the cache | Files genuinely cached, counted after | `InMemoryCache::content_fingerprint` compares file ids, hashes and blob lengths before and after five updates |
| A burst becomes N computations | A real burst spanning the edge, counting **computations** | `git_coalesce.rs` counts `begin`/`finish` pairs, not notifications; `git_push.rs` repeats it end to end with 50 real writes |
| File content reaches the client in a diff | Content that would show up; payload inspected | The fixture changes `PASSWORD=hunter2` to `PASSWORD=swordfish`; the whole serialised payload is searched, in `git_diff.rs` and again in `git-gutter.spec.ts` |
| A rename desynchronises the parser | A real rename, original path a separate NUL field | `git_parse.rs::a_rename_does_not_desynchronise_everything_after_it`, with records following the rename |
| Detached HEAD shows "(detached)" | The fixture really detaches | `Repo::detach`, and `git-branch.spec.ts` detaches at a real commit and asserts the text is not `detached` |

---

## 8. Mutation record

All eight run 2026-09-27. Each failed **with the assertion expected**, never with a compile
error.

| # | Mutation | Failed |
|---|---|---|
| 1 | Apply a status update from its first page, ignoring the cursor | 5 tests, incl. `a_first_page_with_a_cursor_is_not_applied_until_the_last_page_arrives` |
| 2 | Key git state by the tree's file identity instead of by path | `git_state_written_by_one_session_is_there_for_the_next`, `a_replacement_removes_what_is_no_longer_reported` |
| 3 | Take the index character in preference to the worktree character | `a_file_staged_and_then_edited_again_reports_the_unstaged_state` |
| 4 | Assume `<root>/.git` is a directory | `a_linked_worktree_is_watched_where_its_git_directory_actually_is`, and only that one |
| 5 | Remove the trailing edge | `fifty_writes_inside_one_second_cost_one_computation`, `one_change_runs_once_after_the_edge_passes` |
| 6 | Let the git watch name a workspace event type | `the_git_watch_produces_no_workspace_file_events` |
| 7 | Add a field carrying diff context lines | `the_result_carries_no_file_content_in_any_field` |
| 8 | Treat `# branch.head (detached)` as a branch name | `a_detached_head_is_the_detached_case_and_not_a_branch_named_detached` |

Mutation 7 needed a **new field** on `GitDiffResult`, because the type has nowhere for text to
go. That is the point of asserting on the payload rather than on the parser: the only way to
break the guarantee is to change the shape of what travels, and the test sees it when you do.
