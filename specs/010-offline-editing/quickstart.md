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
| **SC-009** interactive latency during prefetch | `cargo test -p apex-shell --test prefetch_budget -- --nocapture` | within 10% |
| **SC-010a** cached files evicted by prefetch | same test | **0** |
| **SC-011** reconciling 100 files | `cargo test -p apex-shell --test reconcile_budget -- --nocapture` | < 10 s |
| **SC-012** requests issued reading a cached file offline | offline-state spec | **0** |

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

Mutations 2 and 3 matter most: each is a plausible simplification that leaves every other test
green, and each corresponds to a decision a reviewer made deliberately.

---

## 6. Validation record

*(Filled in when the feature is implemented. Numbers, not verdicts.)*

---

## 7. Negative-check audit

*(Filled in when the feature is implemented: for each check in §4, the evidence that its condition
is actually present.)*

---

## 8. Mutation record

*(Filled in when the feature is implemented: for each mutation in §5, what failed and whether it
failed with the assertion expected rather than with a compile error.)*
