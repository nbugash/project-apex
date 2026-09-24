# Analysis — F004 file-watch-sync

## Pass 4 — 2026-09-24

Families run: requirement-coverage, constitution-MUST-walk, duplication, ambiguity,
underspecification, inconsistency, deterministic-checks, contract-guarantees, stale-artifact,
dependency-soundness, quickstart-traceability, cross-artifact-field-matching,
spec-internal-contradiction.

Findings: 0 outstanding. Twenty-nine were raised across four passes and all were fixed; the
last pass's six were all caused by the previous pass's own fixes, which is what prompted
`check_propagation` in `scripts/pipeline.py`.

VERDICT: CLEAN

## Implementation record — 2026-09-24

Mutation checks run against the shipped code, each reverted after confirming the failure.

| Mutation | Result |
|---|---|
| Coalescing window 100 ms → 10 s | 12 of 13 coalescer tests fail |
| Deadline reset on every event (debounce rather than throttle) | 3 fail — the lower bound is what catches it |
| Leading edge: keep the first write's state | 1 fails |
| Bulk threshold 256 → 1 | 12 fail |
| Unpaired move halves dropped instead of classified | 2 fail |
| Frame writer lock released mid-frame | 1 fails |
| `inotify` confinement: prefix guard removed | caught |
| Propagation checker: prefix guard, spec exclusion, uncommitted exemption, newest-first, added-lines-only | all 5 caught |

The window mutation is the one quickstart.md flags as most likely to expose a missing
assertion, because FR-012's requirement is an **upper** bound: widening the window makes the
delivered count fall, so a suite with no lower bound passes hardest exactly when the developer
is being told nothing. T016 asserts both ends.

### Defects found by implementation rather than by review

Seven, none of which any analysis pass had surfaced:

1. The exclusion set excluded a directory and none of its contents — it checked whether an
   ancestor matched and then still demanded the path itself match.
2. `FsEntryWire.modified` is Unix seconds; the data model specified milliseconds for the same
   field on the event, which would have made "the same metadata a listing returns" false.
3. A paired rename started its coalescing window when the pair completed rather than when the
   first half arrived, lengthening the window for every rename.
4. The subtree rename rewrote `parent_path` with arithmetic that skipped the separator, making
   a renamed subtree unreachable while every row still looked right.
5. The `inotify` confinement guard fired on three files that named the library only in prose
   explaining why they must not use it, and then on the composition root — resolved by moving
   the factory into the adapter rather than adding a judgement call to the rule.
6. `lint:ds` refused a 6px literal for the tab marker; the value belongs to the prototype, so
   `ds-sync` now extracts `--vk-tab-dot` from it.
7. The mock daemon's guard failed the build on a doc comment that named a §4.8 method in order
   to explain why the directive must not.
