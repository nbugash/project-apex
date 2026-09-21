---
name: speckit-featuremap-add
description: Add a feature to the map with the correct identity, position and dependencies
compatibility: Requires spec-kit project structure with .specify/ directory
metadata:
  author: nbugash
  source: featuremap:commands/speckit.featuremap.add.md
---

# Add a Feature to the Map

Append a feature to `specs/features-map.md` without breaking its invariants: identities are
never reused or renumbered, file position conveys build order, and a feature may never
depend on work positioned below it.

This command edits the map only. It does not write a specification. The new feature is
specified later, through the normal specify command, once its dependencies are complete.

## User Input

$ARGUMENTS

## Step 1: Read the map

```bash
python3 .specify/extensions/featuremap/scripts/python/feature_map.py verify
```

If the map reports integrity problems, stop and report them. Adding to an inconsistent map
propagates the inconsistency.

If it reports `skipped`, no map exists yet. That is not an error here: this command creates
one at `specs/features-map.md` on the first add, and the feature it inserts becomes `F000`.
Say that the map is being created, so the user knows a new artifact appeared.

Otherwise read `specs/features-map.md` to learn the existing tiers, what is already checked,
and where comparable features sit.

## Step 2: Derive a proposal

From the user input, derive:

- **Slug** — two to four words, lowercase kebab-case, matching how neighbouring features
  are named. Preserve technical terms and acronyms.
- **Position** — the identity this feature follows, passed as `--after`. Position is a
  claim about build order, not about importance.
- **Dependencies** — the features that must be complete before this one can start. Every
  dependency must be positioned above the insertion point. Declare what is genuinely
  required rather than everything that happens to come first: over-declaring serialises
  work that could otherwise run in parallel.
- **Parallel marker** — pass `--parallel` when the feature is independent of its immediate
  neighbours.
- **Subfeatures** — three to six mergeable slices, each a scope boundary rather than a
  task. Task breakdown happens later in the workflow; duplicating it here creates a second
  backlog that drifts.

## Step 3: Ask only what cannot be derived

Ask at most three questions, and only where the answer changes the outcome:

- **Position and dependencies**, when the description does not make ordering obvious. This
  is the question most worth asking: a feature inserted above completed work becomes the
  next thing the sequence gate resolves, so a wrong position silently reprioritises.
- **Scope boundary**, when the description could reasonably be one feature or several. A
  feature carrying more than about six subfeatures is usually two features.
- **Whether it is genuinely new.** If it overlaps an existing entry, propose adding
  subfeatures to that feature instead, and warn that doing so to an already-checked feature
  unchecks it and returns it to the queue.

State every inference in the preview instead of asking about it.

## Step 4: Preview, then write

```bash
python3 .specify/extensions/featuremap/scripts/python/feature_map.py add \
  --slug <slug> \
  --after F0NN \
  --depends F0NN,F0NN \
  --parallel \
  --subfeature "First slice" \
  --subfeature "Second slice" \
  --dry-run
```

Show the rendered block and the assigned identity, say where it lands and why, and get
explicit confirmation. Re-run without `--dry-run` to apply.

The script assigns the identity, refuses a dependency positioned below the insertion point,
re-verifies the file after writing, and rolls the write back if integrity would break. Do
not hand-edit the map to work around a refusal; the refusal is the gate.

## Step 5: Report

State the assigned identity, its position and its dependencies. Then state this
consequence when it applies: if existing features should now depend on the new one, their
`(depends: ...)` lines must be edited by hand. Nothing infers that a new feature belongs in
another feature's prerequisites, and verification detects a wrong dependency but never a
missing one.