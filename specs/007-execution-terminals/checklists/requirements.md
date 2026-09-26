# Specification Quality Checklist: Execution Terminals

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

**16 of 16.** The specification opened with two `[NEEDS CLARIFICATION]` markers and closed with
none, by two different routes.

**FR-013, backpressure, was resolved rather than asked**, because the template reserves a marker
for questions with no reasonable default and this one had one: behave as a terminal already does
and slow the process, since a program writing to a terminal nobody reads from blocks when the
buffer fills. The alternatives are not symmetrical — dropped output is a transcript that is wrong
in a way nothing marks, and unbounded buffering moves a runaway build's cost onto an instance
billed by the hour. How much is buffered before slowing is a stated plan value (FR-013a).

**FR-031, task lifetime across a disconnection, was a judgement stop and was asked.** It decided
how much of F020 `detached-engine` gets built inside F010, which is a scope divergence a reviewer
should agree with before planning rather than after implementing. The answer — tasks survive,
clients reattach — is recorded as **A-TASKLIFE** in the system specification rather than only in
this feature's Clarifications, because a decision binding F020 is one F020 must be able to find.

**Implementation details were removed on the first pass**, and the pattern is the same one F004's
checklist recorded. The draft named the panel library, the specific stop signal, the resource
mechanism and the platform — in the Input line, in two source citations, in two edge cases and in
an assumption. Each is the mechanism the system specification prescribes, which is exactly why it
belongs in `plan.md`: a stakeholder reading this needs to know that a process must believe it is
attached to a terminal, not what provides that.

`pty` survived in the Input line because the feature map uses it; the requirement it became
(FR-002) is written as the observable instead — a process asking whether it is attached to a
terminal is told yes, with colour, progress bars and prompting following from that. That is
testable without naming the mechanism.

`stdout`, `stderr` and `ANSI` were kept deliberately. They name a process's streams and a
published control-sequence standard, in the same way `hash` and `path` are kept elsewhere in this
project. They are vocabulary, not a technology choice — nothing about them selects an
implementation.

**Two values in the Success Criteria are this specification's own choices**, recorded in
Assumptions rather than quoted as though they came from the system specification: 500 ms for
output to appear and for a resize to be observed. §1.4 budgets interactions the developer
*initiates*, and watching a build's output is not one, so no existing target applies. Both are
stated with their reasoning so a later measurement is read against a number somebody chose.

**FR-013 and FR-031 are deliberately incomplete.** Both state that the system must have a
*stated, bounded* behaviour, and then ask what it should be. That is the shape a requirement
takes when the observable is clear and the policy is not — the same shape F004's FR-012 and
FR-015 took before the coalescing window and bulk threshold were fixed in the plan.

One scope boundary is flagged for the reviewer rather than resolved here: **FR-031 overlaps F020
`detached-engine`**. F010 owns what happens to a running task at the moment a connection drops;
F020 owns whether the engine survives the client going away. Choosing "tasks keep running"
decides part of F020 from inside F010, which is a divergence the reviewer should agree with
before planning rather than after implementing.
