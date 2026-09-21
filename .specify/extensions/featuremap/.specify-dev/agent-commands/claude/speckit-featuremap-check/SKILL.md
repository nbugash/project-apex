---
name: speckit-featuremap-check
description: Resolve the feature to specify and refuse the run when its dependencies are incomplete
compatibility: Requires spec-kit project structure with .specify/ directory
metadata:
  author: nbugash
  source: featuremap:commands/speckit.featuremap.check.md
---

# Feature Map Sequence Gate

Runs as a mandatory `before_specify` hook. It decides which feature is being specified,
and refuses the run when that feature's prerequisites are not complete.

It also carries the `ADD` mode: a specify invocation whose input begins with `ADD` is a
request to extend the backlog, not to write a specification.

## User Input

$ARGUMENTS

## Prerequisites

The feature map lives at `specs/features-map.md`. It is optional. When it is absent, every
action below reports `skipped` and exits zero, and specification proceeds exactly as it
would without this extension.

The script is installed at `.specify/extensions/featuremap/scripts/python/feature_map.py`.

## Step 1: ADD mode

Inspect the input the user passed to the specify command. If it begins with `ADD`
(case-insensitive), this is a backlog change:

1. Invoke the `speckit.featuremap.add` command with the remainder of the input as its
   arguments. Construct the invocation the way this agent invokes commands, replacing dots
   with hyphens where that applies.
2. Stop. Do not create a specification directory, and do not continue into the specify
   workflow. Adding a feature to the backlog and specifying a feature are different acts;
   a new backlog entry is specified later, when its dependencies are complete.

Otherwise continue to step 2.

## Step 2: Resolve the target feature

Run the resolver from the project root. Pass the feature identity when the user supplied
one, and omit it otherwise:

```bash
python3 .specify/extensions/featuremap/scripts/python/feature_map.py resolve [FEATURE_ID] --json
```

Read the `STATUS` field and act on it:

- **`ready`** — report the resolved `FEATURE_ID`, `SLUG` and pending subfeature count, then
  allow specification to continue. The subfeature list is the scope of the specification
  that follows, so derive the feature description from the map entry rather than inventing
  one.
- **`blocked`** — stop. Report `REASON` verbatim and name every entry under `BLOCKED_BY`
  or `WAITING`. Do not create a specification directory and do not offer to bypass the
  gate. The correct next action is to finish the blocking feature.
- **`error`** — stop and report the problem. An integrity failure means the map
  contradicts itself: a feature checked while its subfeatures are not, a duplicate
  identity, an unknown dependency, or a dependency positioned below its dependent. Each
  makes every later sequencing decision unsound, so it is fixed before work continues
  rather than worked around.
- **`skipped`** — no map exists. Report nothing beyond a single line and let specification
  proceed normally.

Exit codes mirror the status: `0` for ready or skipped, `2` for blocked or an integrity
failure, `1` for a usage or I/O problem.

## Step 3: After the specification is created

Record the resulting directory in the map, so that the map's identities and Spec Kit's
sequential specification numbers stay reconcilable:

```bash
python3 .specify/extensions/featuremap/scripts/python/feature_map.py record-spec F001 specs/002-plugin-host-runtime
```

## Related

- `feature_map.py verify` checks map integrity on its own.
- A project constitution may define this gate as a governing principle. When it does, the
  constitution is the authority on what a checked box means; this command only enforces it.