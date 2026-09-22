# Specification Quality Checklist: SSH Transport Core

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-22
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

Two items needed a judgement to pass, and the judgement is recorded rather than hidden:

**"No implementation details"** — the specification names SSH, a connection, frames and
request identifiers. These are not implementation choices being made here; the transport
mechanism was decided in Appendix A, A-B1 and made normative in §3.1 before this feature
existed. The specification refers to that decision and does not restate the invocation, the
flags or their reasons. Writing around the word "SSH" would have produced a vaguer document
about the same fixed thing.

**"Success criteria are technology-agnostic"** — SC-011 names a round-trip time and a packet
loss rate. They come from the feature map's own subfeature ("Mock SSH daemon harness
simulating 250ms RTT and 5% packet loss"), which is to say from the backlog this feature was
drawn from, and they describe the conditions a measurement is taken under rather than the
technology taking it.

**Settled by clarification on 2026-09-22.** The two defaults previously flagged here — the
latency budget and the request time limit — were confirmed, and two further gaps surfaced
that the first pass had missed entirely:

1. **Reconnection was unspecified.** The spec detected a lost connection and said nothing
   about recovering it. F001 now owns reconnection with backoff (FR-020, SC-012); F012 adds
   the offline experience on top.
2. **The priority queue was unassigned.** §4.6 of the system specification makes it
   normative, and no feature in the map had it. F001 now owns it (FR-021, SC-013).

Neither was a vague requirement — both were _absent_, which is the failure this checklist is
weakest at catching: every item can pass while something nobody wrote down is missing. Worth
remembering the next time this list reads 16 of 16 on a first draft.

**One item flagged for the feature map, not for this spec.** F012 currently carries a
subfeature reading "Connection state detection from keepalive expiry and pipe EOF", which is
now FR-004 and FR-020 here. The map needs rewording so F012 consumes this feature's
connection state rather than re-detecting it.
