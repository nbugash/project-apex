# Analysis — F012 offline-editing

Twenty-eight passes. This file did not exist for the first twenty-seven of them, which is the
finding pass 28 opened with — `scripts/pipeline.py` derives the phase from the artifacts and
records analysis convergence here alone, so `pipeline.py next F012` answered `analyze` after every
one of them and a headless run would have looped forever.

## Families run

Requirement coverage in both directions; constitution MUST-walk across all eight principles;
acceptance-scenario, edge-case, contract-guarantee and success-criterion coverage; refusal-code
coverage against the system specification's catalogue; duplication; ambiguity; underspecification;
cross-artifact consistency; system-specification agreement, including Appendix A in both
directions; registration surfaces (`generate_handler!`, `mod.rs`, `wdio.conf.ts`, `Cargo.toml`,
`migrate.rs`, the webview's outcome union); dependency injection; invocation — what calls each new
type; test quality (vacuity, mutation coverage, negative-check conditions, assertion target);
derived-count drift; task mechanics; the repository's own deterministic checks.

## Passes 1 to 27 — 2026-09-27 and 2026-09-28

Fifty-one findings raised and fixed. Ten of them were created by an earlier pass's own
remediation, which is the pattern F004 met and `check_propagation` exists for.

The ones worth carrying forward, because each names a class rather than an instance:

| Pass | Finding | Class |
|---|---|---|
| 2 | `pending_edits` stored only `base_sha256`; a three-way merge needs the base *text*, and `file_contents` is evictable and overwritten by any refetch | Every design artifact agreed with every other and they were wrong together |
| 11 | No task registered the three new Tauri commands in `generate_handler!` | An artifact exists and nothing points at it |
| 12 | The confinement guard was specified with one half, the vacuous form | A test that cannot fail |
| 13 | Five derived counts restated outside the document that derives them, all stale | Principle II applies to a derived fact |
| 14 | `wdio.conf.ts`'s live `specs` array is order-significant and the task said "add to the list" | A list where position is the requirement |
| 16 | §11.5 and A-PENDING — the authority — still carried the design pass 2 disproved | Twenty-six checks, every one scanning the feature directory |
| 17 | spec.md cited a section number in §11 that has no heading, and A-RECENT was cited by nothing | A substring check that passes on the citation it is validating. The number is written out here rather than in § form, because a quotation of a bad reference is indistinguishable from a bad reference |
| 19 | Nothing invoked prefetch; design.md gave it a precondition, which never causes a call | An emitter with no subscriber, for the fourth time in this project |
| 23 | `buffers.svelte.ts:315` routes every non-`written` outcome to `failed()`, so a retained save reads as a lost one | Assert the outcome, not the store |
| 24 | `-32004` reached no requirement and no test on the path the feature exists for | A refusal the contract promises and nothing delivers |

## Pass 28 — 2026-09-28

Read what `make gate` and `make test` actually run, then ran the repository's own deterministic
checks for the first time.

1. **`analysis.md` was missing.** `check_analyze` reports it as the single problem for this phase.
   The deliverable of an analysis pass in this project is this file with a `VERDICT` line, and
   twenty-seven passes produced commits instead. Written now.
2. **`check_tasks` reported ten `[P]` collisions.** `[P]` means the task writes a file no other
   task writes — the Spec Kit convention says "different files" and `parallel_collisions` enforces
   it, its docstring recording that two such collisions shipped in F003 and survived an analysis
   pass that never cross-referenced paths against the marker. `tasks.md` had invented a second
   meaning for the marker and documented it in prose, which is the second authority Principle II
   refuses. Markers stripped from every task sharing a file; ten remain, each owning its file.
3. **`check_propagation` found `architecture.md` stale.** Its trigger was coarse — a citation of
   FR-016 added in pass 15, not an amendment — but the staleness was real: the file had not been
   rewritten since pass 2, and FR-020b's stale-write refusal was missing from its Data Flow
   sequence while FR-029b's prefetch trigger was missing from its component diagram and table.

The lesson is the same one as pass 16's, one layer out. Twenty-seven passes of hand-written
regexes, and the repository already carried four calibrated checkers for this phase. They are now
the first thing the accumulated check script runs.

VERDICT: FINDINGS

## Pass 29 — 2026-09-28

Ran every deterministic check the project owns rather than more of my own: `pipeline.py`'s four
phase checkers and `check_propagation`, `feature_map.py verify` (consistent, 9 of 21 features
complete), and the cycle skill's open-marker gate (zero live markers before Appendix A). All clean.
Then examined the one thing plan.md records as a deviation and no pass had verified: whether the
design system carries what a conflict panel needs.

One finding, in the class this feature's own quickstart exists to police.

**AD1.** `data-tone` is set on the editor's notice at `EditorPanel.svelte:338` and styled **nowhere**
in the product, so every tone renders identically. quickstart §4's row therefore claimed more than
it proved: it said reading the outcome's tone is what catches a retained save "presented as a
failure", when tone is data and the presentation is decided by whether `save` routes the outcome
through `b.failed()`. The row now names both assertions and says which proves which. T031c says the
same, and T058 records that the design system exposes no semantic tone token — no warning, error or
success colour — so the conflict panel is built from the accent and neutral ramps, which is
sufficient because a conflict panel shows three versions rather than alarming.

Checked and sound: `lint-ds.mjs` does catch a raw hex colour, which I had assumed it did not; it
does not catch `rgb()` or a named colour, which is a gap in shared tooling rather than in this
feature, and T058 now says to follow the design system's rule rather than the lint's reach.

VERDICT: FINDINGS
