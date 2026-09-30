# Quickstart: Offline Editing

**Branch**: `feature/F012-offline-editing` | **Date**: 2026-09-27 | **Plan**: [plan.md](./plan.md)

How to prove this feature works, and how to tell a green suite from a suite that cannot fail.

---

## 1. Prerequisites

```bash
make setup                      # once
cargo build --workspace         # engine and client
```

An engine binary and a real git repository. `make local WS=/path/to/a/repo` runs the application
against one; the end-to-end suite builds its own under `.e2e-workspace`.

---

## 2. The scenario a reader should run first

Offline editing is hard to believe from a test log, so run it once by hand.

```bash
make local WS=/path/to/a/repo          # engine up, workspace open
# edit a file, save it
pkill -x ide-engine                    # the outage
# edit another file, save it; quit the application; relaunch it
make local WS=/path/to/a/repo          # still offline: both edits are present
# let the engine come back, reconnect, and watch what happens to each file
```

What to look for: the editor never locked, both edits survived a relaunch, and reconnection
reported per file what it did rather than leaving you to infer it from the tree.

---

## 3. The measurements

Numbers, not verdicts. Each is printed by the command beside it.

| Criterion | Command | Bound |
|---|---|---|
| **SC-001** connection drop to offline state visible | `npm run e2e:live -- --spec tests/e2e/offline-state.spec.ts` | < 2 s |
| **SC-002** saved offline edits surviving relaunch | same spec | **100%** of ≥50 edits |
| **SC-004** interactions for a clean reconnection | `npm run e2e:live -- --spec tests/e2e/offline-reconcile.spec.ts` | **0** |
| **SC-006b** merge decisions matching `git merge-file` | `cargo test -p apex-shell --test merge_agreement -- --nocapture` | **100%** of ≥20 pairs |
| **SC-007** offline open of a cached file | `cargo test -p apex-shell --test offline_budget -- --nocapture` | < 200 ms |
| **SC-008** offline path search over 50,000 paths | same test | < 1 s |
| **SC-009** interactive p99 during prefetch | `cargo test -p apex-shell --test prefetch_budget -- --nocapture` | ≤ idle p99 + one 64 KiB prefetch response, within 10% (amended) |
| **SC-010a** cached files evicted by prefetch | `cargo test -p apex-shell --test prefetch -- --nocapture` | **0** |
| **SC-011** reconciling 100 files | `cargo test -p apex-shell --test reconcile_budget -- --nocapture` | < 10 s |
| **SC-012** requests issued reading a cached file offline | offline-state spec | **0** |
| **SC-002a** reconciliations using the host-confirmed base, over 10 files saved 3 times each | `cargo test -p apex-shell --test retain_edit -- --nocapture` | **100%** |
| **SC-003** offline edits lost across edit, disconnect, relaunch, reconnect | `npm run e2e:live -- --spec tests/e2e/offline-conflict.spec.ts` | **0** |
| **SC-005** interactions for a non-overlapping host change | `npm run e2e:live -- --spec tests/e2e/offline-reconcile.spec.ts` | **0** |
| **SC-006** overlapping changes resolved by the client choosing a side | `cargo test -p apex-shell --test reconcile` | **0** |
| **SC-006a** unmergeable files prompting, including those the host did not change | same test | **100%** |
| **SC-010** manifests and recent-commit files readable offline once prefetch reports done or stopped | `cargo test -p apex-shell --test prefetch -- --nocapture` | **100%** of what it reported fetching |

Record each in the *Validation record* below when the feature is implemented.

---

## 4. The negative checks

Each must be able to **fail**. The condition beside it is what makes that possible; a check whose
condition is absent passes for any implementation, which is the failure mode this table exists to
catch. F011 shipped three such checks and found them only by mutation.

| Must not happen | The condition that lets it fail |
|---|---|
| A saved offline edit is lost across a relaunch | The application is genuinely **quit and relaunched**, not merely reloaded, and the assertion reads the store rather than a live projection |
| Adjacent-line changes merge silently (FR-020a) | The corpus genuinely contains a pair whose changes are on **neighbouring lines**, since that is the case a zero-context merge gets wrong and every other case would not |
| An unmergeable file is merged | The fixture is genuinely **not held as text** — a real binary, not a text file called `.bin` |
| An unmergeable file that the host did not change is silently written (FR-025a) | The host side is genuinely **untouched**, so a "prompt only when the remote moved" implementation visibly skips it |
| Prefetch evicts something the developer opened | The cache is genuinely **at its budget** before prefetch runs, and the opened file is genuinely the least recently used |
| A conflict is lost by going offline again (FR-025) | The second disconnection is real and the conflict is **left unresolved**, not resolved and re-created |
| Reconciliation writes a file it should have prompted about | The assertion reads the **host's bytes**, not the client's report of what it did |
| An interrupted reconciliation loses work (FR-028) | The interruption lands **between two files**, which needs more than one file with retained work |
| A stale-base refusal during reconciliation is reported as a failure (FR-020b) | The host moves **between** the read and the write of one reconciliation, not before it starts — moving it earlier exercises the ordinary overlap path, which passes either way |
| A retained offline save is presented as a failure | Two assertions, because one proves less than it looks. `describeOutcome`'s label and tone are data and nothing renders `data-tone` today, so asserting them proves the **decision**; what proves the **presentation** is asserting that `save` does not route the outcome through `b.failed()` — `buffers.svelte.ts:315` sends every non-`written` outcome there and compiles either way |
| A deleted workspace root answers with an empty list instead of `-32009` | The directory is genuinely **deleted** while the workspace stays registered, which is the one case that must refuse where a non-repository must succeed |
| Prefetch never runs for a workspace opened after startup (FR-029b) | The workspace is genuinely opened **after** the application started and after a connection already existed, which is the case a startup-only trigger misses while every other prefetch test passes |
| The merge library escapes its adapter | The guard asserts the adapter **does** name `diffy` as well as that nothing else does, so removing the adapter fails the guard instead of satisfying it |

---

## 5. The mutation checks

Break the property, confirm the test fails, restore. A test that passes both ways is not a test.

1. **Delete the pending edit before the host confirms the write.** The interrupted-reconciliation
   test must fail on lost work, not on an error.
2. **Merge with zero context instead of git semantics.** The adjacent-lines case must fail — and
   only that case, which is what proves the corpus is testing the boundary rather than the
   algorithm.
3. **Prompt only when the remote moved.** The unmergeable-file test must fail; every other
   reconciliation test must still pass.
4. **Let prefetch evict.** SC-010a must fail.
5. **Take offline state from a request failing rather than from the connection state.** The
   offline-state spec must fail on latency or on a false positive during a slow request.
6. **Retain unsaved buffers as well as saved ones.** The relaunch test must fail on an unsaved
   buffer reappearing, which is the half FR-011a decides.
7. **Key pending edits by `file_id` instead of by path.** The file-created-offline test must fail.
8. **Resolve a conflict by preferring the local side.** The conflict test must fail without any
   prompt being shown.
9. **Read the base from `file_contents` instead of from the pending edit.** The evicted-cache
   test must fail. **Measured during implementation: five of thirteen fail, and that is correct.**
   The prediction "and only that one" was too strong — the suite also contains a file created
   offline, a file the host deleted, and a file in a workspace whose root is gone, none of which
   has a cached base either. What matters is the contrast, and it holds: the eight tests whose
   base *is* in `file_contents` pass either way, because there the two sources agree. Getting that
   contrast took fixing a fixture — the helper that cached a base called `put_listing` once per
   file, which replaces a parent's children, so every multi-file test was quietly uncached and the
   mutation failed eight tests instead of five.
10. **Re-derive the base from the new local content on a second offline save.** The
    base-preservation test must fail on the stored base, not on a merge result.
11. **Report a `-32004` refusal as `Failed` instead of `Conflicted`.** The stale-base test must
    fail on the outcome. The pending row survives either way, so a test asserting only that the
    work is still there passes both — which is why FR-020b's test reads the outcome.
12. **Remove `diffy` from the adapter entirely.** The confinement guard must fail. A guard
    carrying only the "no other file names it" half passes here, having found nothing to
    complain about, which is how F011 shipped three guards that asserted nothing.

**Mutation 10 matters most, and 2, 3 and 9 after it.** Each is a plausible simplification that
leaves every other test green. Mutation 10 is worse than the rest in kind: the others make a test
fail or a feature go quiet, whereas re-deriving the base makes the merge compare local against
local and return a **clean merge that is wrong**. No assertion about success notices, and the
developer is handed a file nobody wrote.

---

## 6. Validation record

Recorded 2026-09-30 on the development machine (Linux, debug builds, local engine), each from the
command §3 gives. A clean build: the first pass was discarded after a mutation run left a stale
binary behind (§8's note).

| Criterion | Measured | Bound |
|---|---|---|
| **SC-001** drop to offline state visible | **66 ms** | < 2,000 ms |
| **SC-002** saved offline edits surviving relaunch | **10 of 10 files, 50 saves** (100%) | 100% of ≥ 50 |
| **SC-002a** reconciliations using the host-confirmed base | asserted, 10 files × 3 saves (100%); not printed | 100% |
| **SC-003** offline work lost across the full cycle | **0** (1 of 1 kept, through a real second outage and a relaunch) | 0 |
| **SC-004** interactions, clean reconnection | **0** | 0 |
| **SC-005** interactions, non-overlapping host change | **0** | 0 |
| **SC-006** overlaps resolved by the client choosing a side | **0** (no write for the file; asserted) | 0 |
| **SC-006a** unmergeable files prompting, host untouched | **100%** (asserted) | 100% |
| **SC-006b** merge decisions matching `git merge-file` | **22 of 22 pairs** | 100% of ≥ 20 |
| **SC-007** offline open of a cached 64 KiB file | **p99 0.066 ms** | < 200 ms |
| **SC-008** offline path search, 50,000 paths | **p99 3.49 ms** | < 1,000 ms |
| **SC-009** interactive p99 during prefetch *(amended)* | **9.9 ms** during against **18.1 ms** bound (idle p99 0.98 ms + one 64 KiB prefetch response 15.5 ms, +10%); median 0.46 ms idle, 0.66 ms during | ≤ bound |
| **SC-010** manifests and recent files readable once prefetch reports | **4 of 4** reported, all readable offline | 100% |
| **SC-010a** cached files evicted by prefetch | **0** (stopped at the budget; the opened file still cached) | 0 |
| **SC-011** reconciling 100 files | **8 ms** (50 fast-forwards, 50 merges) | < 10,000 ms |
| **SC-012** requests reading a cached file offline | **0** (asserted in the offline-state spec) | 0 |

**Headroom worth knowing.** Everything but SC-009 sits orders of magnitude inside its bound. SC-009
is the tight one, and its 64 KiB response time is dominated by base64 and JSON in a *debug* build;
before the amendment, whole-file prefetch reads measured an interactive p99 of 22.5 ms against an
idle 138 µs, which is what the amendment in spec.md records.

---

## 7. Negative-check audit

For each check in §4, where its condition is actually present.

| Check | Evidence |
|---|---|
| Offline edit lost across relaunch | `offline-state.spec.ts` T027: `relaunch()` ends the session and starts a new application process, launched still offline by the hold file; the count is read from `offline_status`, i.e. the store, not a live projection |
| Adjacent lines merge silently | `merge_agreement.rs`'s corpus has "adjacent lines"; mutation 2 fails exactly that pair (and one artefact of the mutant itself) |
| An unmergeable file is merged | `an_unmergeable_file_prompts_even_when_the_host_has_not_moved` uses PNG-headed, non-UTF-8 bytes on every side (changed during this audit: it was text flagged unmergeable); `every_conflict_has_three_sides...` uses `\xff\x00` content |
| Unmergeable, host unchanged, silently written | Same test: the host holds exactly the base; mutation 3 fails that test and no other |
| Prefetch evicts something opened | `at_its_budget_prefetch_stops_and_evicts_nothing`: the budget is 100 bytes with 60 already cached by a file opened first, the first manifest (30) fits and the second would not. **Recorded, not hidden:** the cache has no size-based eviction at all (§5.5 evicts by age), so "least recently used" cannot be exercised; the budget itself is A-PREFETCHCAP, added during implementation |
| Conflict lost by going offline again | `offline-conflict.spec.ts`: the second outage ends the engine, the relaunch is held offline, the host file is byte-compared at every step and the conflict is resolved only at the end |
| Reconciliation writes what it should prompt about | The e2e specs read the host's file from disk; the Rust tests read `ScriptedHost`'s own write log |
| Interrupted reconciliation loses work | `an_interruption_between_files_reports_not_attempted` retains two files and drops the connection at the second |
| Stale-base refusal reported as a failure | `a_stale_base_refusal_is_reported_as_a_conflict`: the read succeeds and only the write is refused (`refuse_stale`) |
| Retained save presented as a failure | `offline-presentation.test.ts` asserts `save` does not route `heldLocally` through `b.failed()` as well as the label and tone |
| Deleted root answers an empty list | `an_unregistered_workspace_and_a_gone_root_are_refused_differently` deletes the registered directory and expects `-32009` |
| Prefetch never runs for a later workspace | `prefetch_runs_only_while_connected_and_for_any_workspace_opened_later` registers the workspace after the prefetcher exists and while connected; in the application the log shows `prefetch: 1 files cached` on opening a workspace with a manifest |
| Merge library escapes its adapter | `merge_confinement.rs` asserts the adapter **does** name `diffy`; mutation 12 fails it |

---

## 8. Mutation record

Every mutation compiled and failed on an assertion; none failed with a compile error.

| # | Mutation | What failed |
|---|---|---|
| 1 | Forget the row before the write is confirmed | `a_row_survives_a_write_that_was_not_confirmed`, `a_stale_base_refusal_is_reported_as_a_conflict`. **Not** the interruption test §5 predicted: an interruption between files lands at the next file's *read*, so that file never reaches the write this mutation moves. The FR-022 test is the one that locates it |
| 2 | Zero-context merge | `every_pair_agrees_with_git_merge_file` on "adjacent lines" and on "no trailing newline on one side" (the second an artefact of the mutant's own line joining), plus the boundary and region tests |
| 3 | Prompt only when the remote moved | `an_unmergeable_file_prompts_even_when_the_host_has_not_moved`, and nothing else |
| 4 | Let prefetch evict (expressed as ignoring the budget, since the cache cannot evict by size) | `at_its_budget_prefetch_stops_and_evicts_nothing` |
| 5 | Offline from a failed request, not from the connection state | `offline-state.spec.ts` "marks a folder it never listed as unavailable": no request fails for connection reasons, so the mutant never goes offline -- the false negative rather than the latency §5 names |
| 6 | Retain unsaved buffers | `offline-state.spec.ts` "does not retain a buffer the developer never saved" |
| 7 | Key pending edits by `file_id` | `pending_store.rs` (six tests, including `a_path_with_no_file_row_and_no_cached_content_can_carry_work`) and three `cache_contract.rs` tests |
| 8 | Resolve by preferring the local side | `overlapping_changes_prompt_write_nothing_and_keep_the_work`, `reconciliation_is_per_file`, `the_reconciler_asks_the_port_rather_than_deciding` |
| 9 | Base from `file_contents` | Five of 21: the eviction test and the four whose base is not cached either (deleted on host, root gone, per-file, port-decides). The sixteen whose cached base equals the stored one pass, which is the contrast that matters |
| 10 | Re-derive the base on a second save | `a_second_offline_save_replaces_the_content_and_keeps_the_base` (SQLite) |
| 11 | `-32004` as `Failed` | `a_stale_base_refusal_is_reported_as_a_conflict` |
| 12 | Remove `diffy` from the adapter | `diffy_is_named_in_exactly_one_file`, and the agreement suite |

Beyond §5, run during implementation and killed: resolving against a remote re-read at resolve time
(`a_stale_resolution_becomes_a_new_conflict_against_the_newer_remote`), listing every pending row
as a conflict, accepting a resolution with markers, prefetch without listing ancestors, without its
connection gate, without the mid-read digest check, whole-file prefetch reads (SC-009's bound), and
the engine's commit cap, frame truncation and deduplication.

**A stale-binary hazard, found here.** The mutation script restored each file by renaming its backup
back, which keeps the backup's older modification time, so cargo saw no change after the last
mutant and kept that mutant's build. The next measurement ran against a zero-context merge and
reported 20 of 22 pairs. Every mutation result above stands -- each mutant was freshly written, so
each rebuilt -- but anything measured after a restore needs `touch` first.
