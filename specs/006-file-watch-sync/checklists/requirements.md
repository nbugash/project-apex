# Specification Quality Checklist: File Watch Sync

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-24
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

Three items failed on the first pass and were fixed rather than argued down.

**No implementation details.** The draft named `inotify` three times — in the Input line, an edge
case and an assumption. It is the mechanism §10.3 prescribes, which is exactly why it belongs in
the plan and not here: a stakeholder reading this needs to know the host limits watch capacity per
user, not what the system call is called. Replaced with capability language.

**Requirements are testable and unambiguous.** Two requirements carried an adjective where an
observation was needed, which is the fourth and fifth instance of this pattern across F002, F003
and now F004:

- FR-012 said repeated changes "within a short interval" collapse. Short is not a bound. It now
  requires the collapsing window to be a stated duration fixed in the plan, and states the testable
  consequence: the count of events is bounded by elapsed time rather than by writes.
- FR-015 said a change affecting "more paths than a stated limit" becomes one invalidation, and
  nothing stated the limit. §10.4 names the cases and gives no number, and "thousands" is not a
  bound a test can assert against. It now requires a stated count, so the rule is decidable from
  the number of affected paths.

Neither number is invented here. Both are plan decisions, and the requirement is that they exist
as values rather than as judgements — the same shape F003's FR-025 ended up in after review.

**Success criteria are measurable.** SC-001's two seconds was this specification's own choice and
read as though it were quoted. §1.4 budgets interactions the developer *initiates*; a change
arriving from elsewhere is not one and has no existing target. Recorded in Assumptions as a chosen
value with its reasoning, so a later measurement is read against a number somebody chose.

One scope boundary worth flagging to planning rather than resolving here: **§4.8 defines
`workspace/onFileEvent` and `workspace/invalidateAll` as notifications but no `workspace/watch`
request**, while §6.1's trait declares `watch()`. §10.3 reads as though the engine watches the
whole workspace from registration and the client merely filters — in which case no request method
is needed and §6.1's `watch()` is a local subscription. That is a planning question, and it is the
fifth absence of this kind the catalogue has produced.

**Resolved by clarification, 2026-09-24.** It is no longer open. Watching is scoped to what the
developer has expanded plus what they have open, and the engine cannot know either without being
told — so `workspace/watch` is a real request method, not a local subscription, and §6.1's
`watch()` maps onto it. The spec's Clarifications section records the reasoning; the plan decides
the shape.
