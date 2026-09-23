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

### Re-validated after clarification, 2026-09-23

Three questions asked and integrated. All 16 items still pass. Two would have regressed.

**"Requirements are testable and unambiguous"** would have failed on two new requirements, both
written as adjectives rather than observations. FR-018a said a migration must be "visible"; one
message at the start satisfies that and still leaves an upgrade appearing to hang. FR-021b said
the interface must "be able to show" verification is running — a capability rather than a
behaviour, and a capability nothing exercises is not observable. Both now specify a published
state with a cadence, and SC-013a asserts on the number and spacing of reports rather than on one
having been sent.

That is the **fourth** time in this feature, and the sixth across F002 and F003, that an adjective
reached a first draft in place of an observation. It is worth treating as a review question
rather than a coincidence.

**"All functional requirements have clear acceptance criteria"** would have failed on migration.
FR-018a through FR-018c arrived from the clarification and belonged to no user story — the five
stories covered browsing, reading, addressing, bounding and offline, none of which is an upgrade.
User Story 4 was widened from "keep the cache from growing without bound" to "keep the cache
healthy across time", which is what it was already about once migration joined eviction, and
three scenarios were added.

### Decisions taken during clarification that a reviewer should weigh

**Opening a cached file waits for the engine.** Chosen over rendering immediately and
reconciling. The consequence is recorded explicitly in FR-025b: while online the cache saves
transfer, not open latency. Nobody should later read a measurement of that as a regression
against a promise this specification never made.

**Migration happens in place, with a visible state.** Chosen over discarding and rebuilding, so
cached content survives upgrades — at the cost of a migration per schema change, written and
tested against real old caches.

**What a failed migration does was not part of the question and was decided here**: discard and
rebuild, telling the developer. Refusing to launch the application over a cache the specification
itself calls reproducible is not a defensible outcome, and a half-transformed projection is the
one state that could serve wrong bytes while believing they are right.

**Eviction runs once at startup.** A session that never restarts therefore never evicts, and disk
may grow for its duration. Recorded as an accepted limit in FR-026b rather than left to be
discovered: every alternative trigger runs reclaim while somebody is working, and the sidebar
answering in under a millisecond is what §1.4 protects.

### Original pass

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
