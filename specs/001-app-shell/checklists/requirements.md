# Specification Quality Checklist: Application Shell

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-09-21
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

### Amendment 2026-09-21 — dark-only appearance

Re-validated after amending the specification for dark-only appearance under Constitution
Principle I (design fidelity to the signed-off mockup). All 16 items still pass.

The signed-off Nocturne design system is a dark interface and defines no light variant, so the
original requirement to provide both appearances and follow the operating system preference
contradicted the constitution. Changed:

- User Story 4 rewritten from "choose an appearance" to "conform to the approved appearance",
  with four acceptance scenarios covering launch under a light OS setting, non-response to OS
  appearance changes, absence of an unstyled launch frame, and per-surface conformance.
- FR-015 and FR-016 inverted: the application renders the approved dark appearance and MUST
  NOT follow the operating system setting.
- FR-020, FR-021, FR-022 added: no unstyled launch frame; all surfaces derive their styling
  from the approved design; gaps in design coverage are resolved with the designer before the
  surface is built.
- SC-011 and SC-012 added, making both new behaviours measurable.
- Key entity "Appearance preference" removed — no preference exists to model.
- Two edge cases added: operating system set to light, and a required surface the approved
  design does not cover.
- Assumption rewritten to record that a light variant requires design work and fresh sign-off,
  not an implementation decision.

One issue found during re-validation and fixed: the phrase "platform or browser default
styling" leaked the rendering implementation into a specification that is meant to be
technology-agnostic. Reworded to "platform default styling". Re-grepped for technology names
with word boundaries after the fix — zero matches.

SC-007 was listed as conflicting when this amendment was proposed. On inspection it concerns
greyscale distinguishability for colour vision deficiency, which is unaffected by the removal
of a light appearance. It stands unchanged.

### Original validation

Validation performed 2026-09-21. One iteration required.

**Issues found and corrected:**

1. *Requirements are testable and unambiguous* — FR-011 and Acceptance Scenario 2 of User
   Story 3 both specified connection state updating "within a bounded time", which cannot be
   tested. Replaced with 5 seconds.
2. *Success criteria are measurable* — SC-001 specified launch "in under 2 seconds on target
   hardware" with no definition of target hardware, making the threshold unverifiable. Added a
   reference hardware definition to Assumptions and pointed SC-001 at it.

**Checks performed beyond reading:**

- Grepped the specification for technology names that would violate the content-quality items
  (framework, language, library, protocol and component names drawn from the parent system
  specification). Zero matches.
- Confirmed every functional requirement traces to at least one acceptance scenario or
  measurable outcome.

**Deliberate scope decisions**, recorded in Assumptions rather than raised as clarifications,
because each has a defensible default and none blocks planning: single main window, docked
rather than floating regions, two bundled appearances, stubbed connection and workspace state,
placeholder document content.

**Known external dependency**: the connection state this feature displays is produced by the
transport feature, which may not exist when this is built. The specification assumes a stub,
which is what makes this feature independently testable.
