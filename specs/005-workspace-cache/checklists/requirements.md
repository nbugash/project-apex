# Specification Quality Checklist: Workspace Cache

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-23
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

Three requirements failed "testable and unambiguous" on the first pass and were bounded rather
than marked complete. All three were adjectives standing in for observations, which is now the
third time this pattern has reached a first draft in this project — F002 caught it twice.

**FR-024** read "a directory listing MUST be bounded". Nothing can test "bounded". It now
specifies pages of at most 1000 entries. Writing the bound exposed a consequence: a listing of a
hundred thousand entries exceeds §4.1's frame cap by an order of magnitude, so a single-message
listing is not merely slow but undeliverable — and `workspace/readDirectory` in §4.8 has no
pagination. That edit is recorded in the requirement rather than left for implementation to
discover, the way `session/onRestart` was in F002.

**FR-025** said bulk content travels beside the channel without saying what makes content bulk.
The threshold is now the frame cap, so the rule is decidable from a size rather than a judgement.

**SC-010** said cached content occupies "materially less" disk. It now states at most half, over
a representative source tree, with the achieved ratio printed — per A-NFR, a budget only ever
compared against tells nobody how much headroom remains.

### Two scope decisions a reviewer should agree with before planning

**This feature implements the read path only.** §6.1's trait declares write, create, rename,
delete and watch; none are implemented here. Writing brings `baseSha256` conflict handling, which
is F006's subject, and watching is F004's. The trait is declared whole because §6.1 is normative,
and the unimplemented methods refuse rather than pretend.

**It adds workspace methods to the engine.** F002's engine implements none — a test asserts no
§4.8 method name appears in the mock's directory, and the engine itself serves only session
methods. This feature is where the read methods arrive on both sides, which is the same shape as
F002 having to build the engine before it could hand shake with one.

### One obligation inherited rather than invented

F002's plan recorded that Constitution Principle VI's path canonicalisation lands here, because
F002 had no workspace method to apply it to. FR-005 through FR-008 carry it. The engine runs with
the developer's full filesystem rights, so the check must hold independently of anything the
client did — a client-side check protects against bugs, never against a stale or hostile client.
