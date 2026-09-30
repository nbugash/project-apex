# Specification Quality Checklist: Offline Editing

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-27
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

Two items needed a second pass.

**"No implementation details"** initially failed. The first draft of FR-011 named the
`baseSha256` field and the `file_contents` table, and FR-027 named error `-32004`. Those are the
plan's business, not the specification's — a reader deciding whether this feature is worth
building does not need the column name. Rewritten as "the content the client last confirmed with
the host, identified by that content's hash", which is the same requirement and stays testable.
The one place protocol detail survives is the *Superseded subsections* table, where naming §5.2
is the point: it records what is stale.

**"Scope is clearly bounded"** initially failed because nothing said what this feature does
*not* do. A reader could reasonably have concluded that an offline outbox covers arbitrary
operations — offline `git commit`, offline task runs. FR-033 and FR-034 now bound it: content
and its base, nothing else, and no automatic conflict resolution.

Both clarifications recorded in the spec are decisions the reviewer made against the
recommendation, and both are recorded with their cost rather than as settled facts. That is
deliberate: the cost is the part that will matter if either turns out to have been wrong.

## Re-validation after clarify (2026-09-27)

Four further clarifications integrated; **16/16 still passing, no regressions**. Three of the
four sharpened items that were already passing rather than fixing failures:

- *Requirements are testable and unambiguous* was the weakest pass before clarify. "Overlapping"
  carried the whole merge design and was undefined; FR-020a now names context-aware three-way
  merge semantics, and SC-006b makes it checkable against a standard tool rather than against an
  opinion.
- *Success criteria are measurable* improved for the same reason: SC-002 said "offline edits"
  where the answer is that only **saved** edits are retained, so the old wording would have been
  read as a failure by anyone testing an unsaved buffer.
- *Edge cases are identified* gained the two cases the answers created rather than resolved: an
  unsaved buffer when the application stops, and prefetch meeting a full cache.

One answer went against the recommendation — files the client cannot merge stay editable and
always prompt — and it **removed** a restriction rather than adding one, so nothing in the
checklist regressed. FR-017a and FR-025a carry it, and SC-006a makes the "always prompts, even
when the host did not change it" half measurable, which is the half an implementation would
otherwise quietly drop.

## Sample verification (analyze run 18)

The cycle skill requires two items to be confirmed true of the artifact rather than merely
ticked, because a checklist counts items and not whether they hold. Two were sampled:

- **Success criteria are technology-agnostic.** Holds. No success criterion names a tool,
  library or language. SC-006b is the one that could have: it says "matches a standard
  version-control three-way merge", not "matches `git merge-file`". The tool name appears only
  in quickstart.md's measurement command, which is where a command belongs.
- **No implementation details leak into specification.** Holds for requirements and criteria: no
  functional requirement names a table, a crate or a language. `pending_edits` does appear twice
  in this spec — in the `## Clarifications` entry that records the reviewer's answer, and in the
  table of system-specification subsections this feature corrected. Both are records of decisions
  rather than requirements, and a spec that must say which table §5.2 gained cannot avoid naming
  it. The item stays checked on that reading, stated here so the next sampler need not re-derive
  it.

