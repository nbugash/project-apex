<!--
SYNC IMPACT REPORT
==================

--- v1.2.1 (2026-09-21) ---
Version change: 1.2.0 → 1.2.1
Bump rationale: PATCH. Clarification only. No principle added, removed or redefined; nothing
that was required becomes optional or vice versa.

Principle I: the exclusion clause named a specific directory that no longer exists. Replaced
with a path-independent rule — sign-off, not location, decides what binds — which is both
durable and what the clause always meant.

Resolved: the architecture contradiction recorded as conflict 2 under v1.0.0. The contributed
architecture document has been removed from the repository, and the decision it contested is
now recorded as A-UI in the system specification's Appendix A, with rationale, the rejected
alternative and two reversal conditions. Principle II is satisfied for that contradiction.

Still outstanding from v1.0.0: conflict 3, the adherence lint's framework configuration.
Conflict 1 was closed by amending the app-shell specification for dark-only appearance.

--- v1.2.0 (2026-09-21) ---
Version change: 1.1.0 → 1.2.0
Bump rationale: MINOR. One principle added, no existing principle removed, redefined or
renumbered. Constrains how code is organised; forbids no previously-approved decision.

Principles added:
  VIII. Ports and Adapters

Sections modified:
  Development Workflow and Quality Gates — architecture conformance added to the gate list.

Deferred items: none.

Interaction with existing artifacts:
  - project-apex-predator.md §6.1 already defines WorkspaceProvider as a trait with local and
    remote implementations. That is an outbound port with two adapters. This principle
    codifies a decision already taken rather than imposing a new structure.
  - A contributed architecture document (since removed; see v1.2.1) objected to placing a
    SERIALIZATION boundary in the keystroke path. That objection is sound and independent of
    its authorship, which is why Principle VIII states explicitly that a port is an interface
    and not a process boundary.
  - Principle V bounds the interaction path. Principle VIII is written not to license
    indirection there.
  - Principle VII's unit-test level becomes easier to satisfy: use cases tested against
    in-memory fake ports is the intended shape.

--- v1.1.0 (2026-09-21) ---
Version change: 1.0.0 → 1.1.0
Bump rationale: MINOR. One principle added, no existing principle removed, redefined or
renumbered. Guidance materially expanded; nothing previously permitted becomes forbidden
except shipping a feature without tests.

Principles added:
  VII. Every Feature Ships With Tests

Appended as VII rather than inserted near the other verification principle (V), so that
principles I-VI keep their numbers and existing references to them stay valid.

Sections modified:
  Development Workflow and Quality Gates — test gate added to the gate list.

Deferred items: none.

Interaction with existing artifacts:
  - project-apex-predator.md carries [OPEN: TEST], which defers the test strategy. That open
    item is now constrained rather than closed: whatever strategy it lands on MUST satisfy
    Principle VII. The mock SSH daemon already specified in §18.1 is an instance of the
    integration level, not a substitute for the other two.
  - Principle V already requires a measurement for the interaction budget. Principle VII does
    not replace it; a performance measurement is not a substitute for correctness tests.

--- v1.0.0 (2026-09-21) ---
Version change: none (unfilled template) → 1.0.0
Bump rationale: MAJOR. First ratified constitution. The prior file was the unmodified
scaffold with every placeholder intact, so this establishes governance rather than
amending it.

Principles added (all new):
  I.   Design Fidelity to the Signed-Off Mockup
  II.  One Source of Truth
  III. Decisions Are Recorded Before Implementation
  IV.  Open Items Block Their Feature
  V.   The Interaction Budget Is Verified, Not Asserted
  VI.  Trust Boundaries Are Enforced on Both Sides

Sections added:
  Design System Compliance  (resolved SECTION_2)
  Development Workflow and Quality Gates  (resolved SECTION_3)
  Governance

Principles removed or renamed: none.

Deferred items:
  None. RATIFICATION_DATE is today, being the first adoption.

Conflicts this constitution creates with existing artifacts, requiring follow-up
outside this command:
  1. specs/001-app-shell/spec.md FR-015, FR-016, User Story 4 and SC-007 require light
     and dark appearances following the OS preference. The signed-off design system is
     dark-only. Principle I makes the mockup authoritative, so the specification must be
     amended.
  2. [RESOLVED 2026-09-21 — see v1.2.1 above] A contributed architecture document
     contradicted project-apex-predator.md on the UI layer, process topology, text buffer
     ownership, hardware budget and target platforms. The document has been removed and the
     decision recorded as A-UI in Appendix A.
  3. mockups/_ds/.../\_adherence.oxlintrc.json is configured for React. The specified UI
     stack is TypeScript with Svelte. The rules are framework-agnostic in substance; the
     plugin configuration needs porting.
-->

# Project Apex Predator Constitution

## Core Principles

### I. Design Fidelity to the Signed-Off Mockup

The graphical interface MUST match the approved prototype in `mockups/` exactly. The
prototype and its Nocturne design system carry stakeholder sign-off; they are the
specification of appearance and interaction state, not a reference or an inspiration.

Binding artifacts are the HTML prototype, the `_ds/` design system bundle, and the bundled
font and icon sets. Sign-off decides what binds, not location: a contributed document that
has not been through design sign-off is not binding under this principle wherever it sits in
the repository, and a document that is not a visual artifact is never binding under it.

Concretely, and testably:

- Every colour, font, spacing, radius and shadow MUST come from a design system token via
  `var(--*)`. Raw hex values, raw pixel values and hard-coded font names are violations.
- Components MUST use the design system's classes rather than parallel implementations.
- Interaction states — hover, pressed, `:focus-visible`, `::selection`, disabled — MUST come
  from the system. Browser defaults are violations, including the default focus ring.
- Deviation requires written designer approval recorded in the feature's specification
  before implementation. "It looked better" is not approval.

Rationale: an interface signed off by stakeholders is a decision already made. Re-litigating
it per component produces drift that nobody approved and that is expensive to unwind late.
Token discipline is what makes the fidelity checkable by a machine rather than by argument.

### II. One Source of Truth

`project-apex-predator.md` is the system specification. Where any other document contradicts
it, the contradiction MUST be resolved in that file before dependent work starts.

A second architecture document is not an alternative view to be reconciled later; it is an
unresolved decision blocking the features it touches. Contributed documents may inform a
decision, but they do not hold authority by existing in the repository.

Rationale: this project has already paid for divergence — it shipped two incompatible cache
schemas and two incompatible protocol contracts in one document, and each cost real work to
find and reconcile. The cheapest time to resolve a contradiction is before anything is built
on either side of it.

### III. Decisions Are Recorded Before Implementation

Any decision that closes a genuine alternative MUST be recorded in Appendix A of the system
specification before the code implementing it is written. A record MUST state the decision,
its rationale, the alternatives rejected and why, and the conditions that would reverse it.

Records are dated and superseded, never edited in place.

Rationale: the rejected alternatives and the reversal conditions are the load-bearing parts.
Without them a future reader cannot tell a deliberate choice from an accident, and re-opens
settled questions. A decision whose reversal conditions are written down can be revisited
cheaply when those conditions occur.

### IV. Open Items Block Their Feature

An `[OPEN: id]` marker in the system specification is a hard gate on the feature it appears
in. Work MUST NOT proceed past it on the assumption it will be resolved later.

The feature map's sequence gate checks whether dependencies are complete. It cannot see
whether a specification has holes, and it will happily report a feature ready whose
prerequisites are undecided. Enforcing this principle is therefore a human responsibility at
`/speckit-specify` time, not something the tooling will catch.

Rationale: an unresolved open item is a decision that will be made anyway — by whoever writes
the code first, implicitly, without review. That is how the contradictions in Principle II's
rationale got in.

### V. The Interaction Budget Is Verified, Not Asserted

The sub-250 ms interaction budget is a test obligation, not a design aspiration. Any feature
that touches the interaction path MUST ship with a measurement that fails when the budget is
breached.

Two rules are absolute. A keystroke MUST render from the local buffer without awaiting the
network. No bulk payload may occupy the control channel in a way that delays interactive
traffic.

Rationale: the entire architecture — a thin client, a remote engine, speculative local echo —
exists to buy this budget. An unmeasured budget is a claim, and this specification has
already been found asserting the budget on one page and contradicting it on another.

### VI. Trust Boundaries Are Enforced on Both Sides

Input crossing a process or network boundary is untrusted at the receiving end regardless of
what the sending end validated.

Specifically: workspace-relative paths MUST be canonicalised and asserted to be descendants
of the workspace root by the engine, independently of any client-side check, and symlinks
resolving outside the root MUST be rejected. The remote engine runs with the developer's full
filesystem rights, so a malformed or hostile frame must not be able to read or write outside
the workspace.

Rationale: the client and engine are separately deployable and separately versioned. A
client-side check protects against bugs, never against a hostile or stale client, and the
blast radius here is the developer's entire machine.

### VII. Every Feature Ships With Tests

A feature is not complete until it carries automated tests at every level where it has
surface, and those tests pass.

- **Unit** — logic exercisable in isolation: codecs, parsers, path resolution, cache validity,
  state machines, status parsing. Required wherever such logic exists.
- **Integration** — behaviour across a real boundary: the protocol against a real `sshd`, the
  cache schema against a real database file, a provider against a real filesystem. Required
  wherever the feature crosses a process, network or storage boundary.
- **End to end** — a complete user journey. The acceptance scenarios in the feature's own
  specification are the source; each one MUST have a corresponding automated test.

Writing tests before the implementation satisfies this principle and is preferred wherever a
wrong answer is expensive — protocol framing, path containment, cache validity, conflict
detection. The obligation is that the tests exist and pass before the feature is called done;
it is not a prohibition on writing them first.

Omitting a level requires a one-line justification in the feature's specification naming the
level and why the feature has no surface there. Schedule pressure is not a justification.

Rationale: the three levels fail differently and none substitutes for another. Unit tests
catch logic errors. Integration tests catch mismatched assumptions between components — the
exact class of defect that put two incompatible cache schemas and two incompatible protocol
contracts into this project's own specification. End-to-end tests catch the case where every
part works and the product still does not. Tests written after the fact tend to encode what
the code does rather than what it was supposed to do, and they never fail first, so they
demonstrate nothing about whether they would have caught the bug; that is why fail-first is
required where correctness is expensive rather than merely nice.

### VIII. Ports and Adapters

Both codebases — the desktop client and the `ide-engine` daemon — MUST be organised as ports
and adapters. Dependencies point inward only: adapters depend on the application layer, the
application layer depends on port interfaces, and the domain depends on nothing external.

- **Every side effect is an outbound port.** Filesystem access, the SSH transport, the SQLite
  cache, process spawning, language server supervision, git invocation, the system keychain,
  the clock. A capability, never a technology: the port is `WorkspaceProvider`, not
  `SqliteWorkspaceStore`.
- **Every entry point is an inbound adapter.** A Tauri command from the webview and a
  JSON-RPC method dispatch in the daemon are both adapters. They translate protocol input
  into use-case input and translate results and errors back. They contain no business rules.
- **Use cases orchestrate and return plain data.** They MUST NOT read protocol envelopes,
  request metadata or database rows directly.
- **Wiring lives in one composition root per binary.** Hidden global singletons and service
  locators are violations.
- **Framework and library types stay in adapters.** Domain and application code MUST NOT
  import Tauri, `rusqlite`, `serde_json` envelopes, Monaco or Svelte types.

In Rust a port is a trait; an adapter is a type implementing it; a use case is a struct
holding its ports and constructed explicitly. In the webview layer, Svelte components are
inbound adapters — the component tree is not itself hexagonal and MUST NOT be forced into
that shape.

**A port is an interface, not a process boundary.** This principle mandates interface seams,
never additional serialization or additional hops. Introducing a process, network or
serialization boundary into the interaction path is forbidden by Principle V and is not
licensed by this one. Where the two appear to conflict, Principle V wins and the seam moves
out of the hot path.

Rationale: the project already depends on this shape working. The same interface serves a
local workspace and a remote one, which is what makes local mode, remote mode and cloud burst
one product rather than three. The same shape is what allows a transport decision to be
reversed — the recorded condition for revisiting the SSH transport assumes the transport sits
behind a seam that can be swapped. And it is what makes the unit-test level of Principle VII
cheap: a use case tested against in-memory fake ports needs no network, no instance and no
database.

## Design System Compliance

These rules operationalise Principle I and are enforceable in continuous integration.

- **Tokens only.** No raw hex colour, no raw pixel value, no hard-coded font family in
  application code. The adherence configuration at
  `mockups/_ds/nocturne-*/\_adherence.oxlintrc.json` encodes these as lint rules and MUST run
  in CI. It is currently configured for React and MUST be ported to the project's actual UI
  framework without weakening the rules it expresses.
- **Typography.** Inter for interface text, JetBrains Mono for code, loaded from the bundled
  font files. No system font substitution, no web font CDN.
- **Iconography.** Phosphor icons only, from the bundled set.
- **Ground.** Nocturne is a dark interface. Any requirement elsewhere in the project for a
  light appearance is superseded by this section until a light variant is designed and signed
  off.
- **Accessibility floor.** The accent-to-ground pair is tuned to at least 3:1, which is
  sufficient for icons, large text and chrome but NOT for body copy; paragraph text in the
  accent MUST use a deep ramp step. Keyboard focus MUST be the design system's 2px accent
  `:focus-visible` ring. State MUST NOT be conveyed by colour alone.
- **Changes to the system itself** are made by editing the design tokens, keeping `theme.json`
  and the written guidance in step. Application code MUST NOT override system styles locally.

## Development Workflow and Quality Gates

- **Specification precedes planning precedes tasks precedes implementation.** The Spec Kit
  flow is the working process, and `plan.md` is the sole source of the technology stack for a
  feature.
- **The feature map governs sequencing.** `specs/features-map.md` is the backlog. File
  position conveys build order. Identities are immutable from the first recorded spec path
  onward; before that point the map may be rebuilt freely.
- **A specification is not done until its quality checklist passes.** Vague criteria are
  defects: a requirement that cannot be tested as written MUST be rewritten before planning.
- **Every feature declares its dependencies narrowly.** Declaring a transitive dependency
  serialises work that could run in parallel. Verification detects a dependency on work
  positioned below, but never a dependency that should exist and does not, so missing
  prerequisites are caught in review.
- **Architecture conformance is checked at plan time.** Every plan MUST name the ports the
  feature introduces or consumes, and which adapters implement them. A feature that cannot
  express its dependencies as ports is a signal the boundary is wrong, not that the principle
  does not apply.
- **Tests gate completion.** A feature is not done, and MUST NOT be checked off in the
  feature map, until its tests exist at every applicable level and pass. Any omitted level
  carries its recorded justification in the specification.
- **Claims require evidence.** A statement that something passes, builds or is complete MUST
  be accompanied by the command run and its output. This applies to the interaction budget
  (Principle V), to design adherence, and to test results.

## Governance

This constitution supersedes other practices and conventions in this project. Where a
specification, a plan or a review comment conflicts with it, this document wins until it is
amended.

**Amendment procedure.** Amendments are proposed as a change to this file, stating the
principle affected, the rationale, and the migration required of existing artifacts. An
amendment that invalidates completed work MUST include what happens to that work. The Sync
Impact Report at the top of this file records every amendment.

**Versioning policy.** Semantic versioning. MAJOR for removing or redefining a principle in a
backward-incompatible way. MINOR for adding a principle or materially expanding guidance.
PATCH for clarifications and wording that do not change what is required.

**Compliance review.** Every specification is checked against these principles before
planning; every plan before tasks; every change before merge. Principle I is additionally
enforced mechanically in CI. A violation is either fixed or promoted into an amendment — it is
never silently accepted, and a repeated exception is evidence the principle is wrong and
should be amended rather than ignored.

**Version**: 1.2.1 | **Ratified**: 2026-09-21 | **Last Amended**: 2026-09-21
