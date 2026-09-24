# Driving the pipeline

Spec Kit gives every feature the same run of phases — specify, clarify, plan, tasks,
analyze, implement. Five features have been through it by hand. Sixteen remain, and the
hand-driving is the part that does not scale: remembering where a feature stands, knowing
which command comes next, and noticing when a phase reported success it had not earned.

`scripts/pipeline.py` does those three things.

```
make next                  where the next feature stands, and the command to run
make next F=F005           the same, for one named feature
make verify F=F004         the deterministic checks for its current phase
make pipeline              what the runner would do, without doing it
make pipeline EXECUTE=1    actually invoke claude, phase by phase, to the next gate
```

## Phase is derived, not tracked

There is no state file recording which phase a feature is in. The phase is read off the
artifacts:

| Present | Phase |
| --- | --- |
| no `spec.md` | specify |
| `spec.md` with no `### Session` under `## Clarifications` | clarify |
| clarified, missing one of the seven plan artifacts | plan |
| planned, no `tasks.md` | tasks |
| tasks, no current clean `analysis.md` | analyze |
| analysis clean, tasks unchecked | implement |
| every task checked | complete |

This is Principle II applied to the pipeline itself. A tracked phase is a second source of
truth that drifts the first time a command is run outside the runner, or a branch is
switched, or a crash lands between writing an artifact and recording that it was written.
A derived phase cannot drift, because there is nothing to drift from.

The single exception is `analysis.md`. Whether an analysis ran, what it examined and what
it concluded is not recoverable from any other artifact, and without it a loop has no way
to tell convergence from exhaustion. F003 took nine analysis passes and the record of all
nine lived in a chat log. Now it is a file.

An analysis goes stale when `spec.md` or `plan.md` changes after it. `tasks.md` is
deliberately not on that list: implementation mutates it by ticking boxes, and counting
that as a design change would hold the loop at analysis forever.

## Two gates belong to a person

The runner stops rather than guessing at:

**clarify** — because the answers are product decisions. In F004, "watch only what's open"
decided that the protocol gains a method, and "any open tab" invalidated two requirements
written under an earlier answer. A self-answered clarification is *internally consistent*,
which is exactly what analysis checks, so a wrong one passes every downstream gate and
surfaces as rework.

**complete** — because ticking a box in `specs/features-map.md` is an evidence claim. It
lands with the reviewed merge, not with the machine that did the work.

## Why the checks are deterministic

Every check in the driver is a claim this project has already seen a model get wrong about
its own work:

- Three tasks were marked complete and un-marked on verification.
- `npm run gate:fidelity | tail -12` printed `EXIT=0`, which was `tail`'s status for a run
  that had timed out.
- An end-to-end suite passed while `FileTree.svelte` was never mounted.

A self-report is not evidence. So the runner advances a phase only when the artifacts say
it did, re-derives the phase afterwards and stops if nothing moved, and runs `make gate`
before letting a feature reach complete.

The `[P]` collision check earns its place on the same argument. Spec Kit defines `[P]` as
"different files, no dependencies"; two tasks marked `[P]` that edit one file are a false
claim. Run against the five finished features it found **seventeen** of them — seven in
F000, seven in F002, three in F018, none in F001 or F003. F002 alone has nine `[P]` tasks
writing `src-tauri/tests/bootstrap_handshake.rs` and six writing `bootstrap_deploy.rs`.

All seventeen shipped, and none of them hurt, because tasks have always been executed one
at a time. That is the point worth keeping: the marker has been decorative, so nothing has
tested it, and it becomes a corruption hazard the first time anything runs `[P]` tasks
concurrently. A regex does not get bored, and it reads the marker as the contract it
claims to be.

It needed calibrating, though, and the calibration is the interesting part. Reading every
path-shaped token as a file the task edits reported two collisions in F003 that were not
collisions: a markdown link to `data-model.md` that two tasks cite for their field
definitions, and a screenshot directory two tasks drop distinct captures into. Citations
and output directories are not edits. `edited_paths()` drops link targets, `./`-relative
paths, `specs/` paths, templated directories and anything without a file extension, and
`pipeline_test.py` pins both of those cases so the loosening cannot come back.

## What it does not do

It does not decide that a feature is finished, write the map, open a pull request, or
answer a clarification. It reports, checks, and advances between checks.

Analysis convergence is capped rather than solved. `MAX_ANALYSIS_PASSES` stops a loop that
is not converging; it does not make the loop converge. What would is a fixed, enumerable
set of check families in the analyze skill, run once each per pass, with the pass declared
clean when a full sweep finds nothing. Until that exists, F003's lesson stands: pass nine
found real gaps after eight clean ones, because it was the first pass to walk the
constitution MUST by MUST. A clean pass means the families that ran found nothing. It does
not mean there was nothing to find.
