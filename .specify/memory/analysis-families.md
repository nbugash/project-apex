# Analysis check families

## The problem this exists to solve

`/speckit-analyze` has no fixed point. F003 was analysed nine times. Passes one through
eight were clean by the end, and pass nine found real gaps — not because the artifacts
changed, but because it was the first pass to walk the constitution MUST by MUST. A new
kind of check found a new kind of problem.

That is the whole difficulty with "loop until no issues are found". The loop terminates
when the checker stops being inventive, not when the artifacts are correct. Both failure
modes are live: a model that always finds one more thing never finishes, and a model that
declares victory finishes too early. Neither is a signal.

## The fix

Fix the families in advance. Each pass runs every family exactly once. A pass is clean
when a full sweep over all of them produces no findings. That is a real termination
condition, because the number of families is finite and known before the pass starts.

A clean pass then means something precise and modest: **the families that ran found
nothing**. It does not mean there is nothing to find. Widening the set is a deliberate
edit to this file, which makes the widening reviewable — unlike a model deciding
mid-loop to look somewhere new.

## How a family is used

`/speckit-analyze` runs each family against `spec.md`, `plan.md` and `tasks.md`, then
writes `specs/<feature>/analysis.md`:

```
## Pass 3 — 2026-09-24

Families run: <every name below>
Findings: 0

VERDICT: CLEAN
```

`scripts/pipeline.py` reads the last `VERDICT:` line and refuses to advance to
implementation on anything but `CLEAN`. It also treats the analysis as stale once
`spec.md` or `plan.md` changes underneath it.

## Required shape

Each family needs a name, the question it asks, and what counts as a finding. A family
whose finding condition cannot be stated is not a family — it is a mood.

## Families

### constitution-conformance

Walk the constitution principle by principle. For each MUST, name the artifact that
satisfies it or record that nothing does.

*Finding*: a MUST with no satisfying artifact, or an artifact that contradicts one.

This family is here because it is the one that found gaps after eight clean passes, and
because Principle IV makes an unresolved item block its feature.

### requirement-coverage

Map every FR and SC to the tasks that implement it, and every task back to a requirement.

*Finding*: a requirement with zero tasks, or a task tracing to no requirement.

