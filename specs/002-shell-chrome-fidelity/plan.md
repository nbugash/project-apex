# Implementation Plan: Shell Chrome Fidelity

**Branch**: `002-shell-chrome-fidelity` | **Date**: 2026-09-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-shell-chrome-fidelity/spec.md`

## Summary

Bring the shell's chrome up to the signed-off prototype: a window chrome header, an activity
rail that navigates between tool windows, and a tool window with the prototype's header and
rhythm. Carry the prototype's dimensions as tokens rather than literals, and add a repeatable
visual comparison so fidelity is proven rather than argued.

This extends F000, which built three generic regions. It adds no network surface and no new
outbound port: tool window state rides the session store that already exists.

## Technical Context

**Language/Version**: Rust 1.75+ (core, edition 2021); TypeScript 5.x (interface layer)

**Primary Dependencies**: Tauri v2; Svelte 5 + Vite; the signed-off Nocturne design system,
Inter and JetBrains Mono, and the Phosphor icon set, all bundled from `mockups/` and copied by
`ds:sync`. ImageMagick for the fidelity comparison — already required by the end-to-end
harness, so it adds no new prerequisite

**Storage**: The existing session file. Tool window state extends `PersistedSession`, which
raises a migration question resolved in [research.md](./research.md), "Extending the persisted
session without discarding it"

**Testing**: `cargo test`; Vitest; `tauri-driver` + WebdriverIO on Linux; plus the fidelity
comparison, which is a new gate rather than a new framework

**Target Platform**: macOS 13+ and Linux. The fidelity comparison needs a rendered window, so
it inherits the platform limitation in Appendix A, A-E2E and runs on Linux only

**Project Type**: desktop-app

**Performance Goals**: Switching tool window destinations produces no visible stall over
100 ms (SC-008); the rail and tool window hold their widths during window resize without
reflow lag

**Constraints**: The prototype is authoritative for every value (spec, "On the source of
values"); comparison tolerances are 2 device pixels of position and 0.5% of area at a
1200×800 reference size; no dimension may appear as a literal at the point of use (FR-009)

**Scale/Scope**: One tool window region on one edge; the rail's destination count comes from
the prototype; single local user

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

Evaluated against constitution v1.2.1.

| Principle | Verdict | Basis |
|-----------|---------|-------|
| I. Design Fidelity | **PASS — and this feature is its enforcement** | The feature exists to close the gap between the shell and the prototype, and adds the mechanical check that token linting cannot provide: `lint:ds` catches a wrong *value*, nothing currently catches a surface forty pixels too wide. One tension is real and resolved in research: the prototype's layout dimensions are inline fallbacks in its markup, not tokens in the design system, and the design system may not be edited. |
| II. One Source of Truth | **PASS** | The spec defers every value to the prototype rather than restating it, precisely so a second source cannot form. |
| III. Decisions Recorded | **PASS** | Feature-local decisions in `research.md`; one candidate for Appendix A promotion is flagged there. |
| IV. Open Items Block | **PASS** | No `[OPEN: …]` marker in the system specification falls inside this feature's scope. |
| V. Interaction Budget | **PASS with obligation** | SC-008 bounds destination switching at 100 ms and is the measurable form of the budget here. Persistence of tool window state must stay off the interaction path, which the existing debounced writer already provides. |
| VI. Trust Boundaries | **PASS** | The feature adds one command surface — setting the active destination — validated at the boundary like every other. No new external input. |
| VII. Tests Ship With Features | **PASS** | Unit for rail state and collapse behaviour; integration for session persistence of the new fields; end-to-end for navigation, keyboard reachability and greyscale distinguishability; plus the fidelity comparison itself. Linux-only end-to-end is the standing limitation, already justified project-wide in A-E2E — cited, not re-argued. |
| VIII. Ports and Adapters | **PASS** | No new outbound port. Tool window state extends the entity the existing `SessionStore` already persists, which is the correct outcome: a feature that fits behind an existing port is evidence the boundary was drawn in the right place. |

### Ports touched by this feature

Required by Principle VIII and the Development Workflow gate.

| Port | Direction | Change |
|------|-----------|--------|
| `SessionStore` | outbound | None to the port. Its persisted entity gains tool window state; the interface is unchanged |
| `ShellCommands` | inbound | One added operation: set the active rail destination |

That the port set barely moves is the point. F000's boundaries were drawn so that a feature
adding a whole surface does not require a new one.

## Project Structure

### Documentation (this feature)

```text
specs/002-shell-chrome-fidelity/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
├── architecture.md      # Phase 2 output
├── design.md            # Phase 2 output
├── checklists/
│   └── requirements.md  # Spec quality checklist
└── tasks.md             # Phase 3 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

Extends the trees F000 established; no new top-level structure.

```text
src-tauri/
├── src/
│   ├── domain/
│   │   ├── rail.rs                    # RailDestination, availability, active state
│   │   └── session.rs                 # (extended) tool window state on PersistedSession
│   ├── application/use_cases/
│   │   └── persist_session.rs         # (extended) set active destination, collapse
│   └── adapters/inbound/
│       └── tauri_commands.rs          # (extended) rail command
└── tests/
    └── session_migration.rs           # Integration: an older session survives the upgrade

src/
├── app.css                            # (extended) imports the generated layout tokens
└── lib/
    ├── ds/layout-tokens.css            # GENERATED by ds:sync from the prototype. Inside
    │                                   #   src/lib/ds so lint:ds's existing skip covers
    │                                   #   the raw pixel values a token file must contain
    ├── chrome/
    │   ├── ChromeHeader.svelte         # Product mark, project switcher
    │   ├── ActivityRail.svelte         # The rail and its destinations
    │   ├── RailButton.svelte           # One destination: icon, active, unavailable
    │   └── ToolWindow.svelte           # Header row, frame, collapse
    ├── rail.ts                         # Ordering and keyboard-navigation helpers
    └── shell/
        └── Window.svelte               # (extended) composes chrome, rail, tool window

tools/gate-fidelity/
├── compare.mjs                        # The comparison: measure, diff, report
├── update.mjs                         # Writes the baseline; a separate command by design
├── gate.test.mjs                      # The gate's own self-tests
├── reference/
│   ├── derivation.md                  # How the baseline's geometry was derived from the
│   │                                  #   prototype, and the verification against it
│   └── shell.json, shell.png          # Approved baseline; golden files
└── out/                               # Run output; gitignored

tests/
├── unit/rail.test.ts
└── e2e/chrome-fidelity.spec.ts
```

Individual end-to-end spec files are not enumerated. They are test artifacts whose names
follow from the scenarios they cover, and listing each would churn this tree on every test
added while telling a reader nothing about the system's shape.

**Structure Decision**: Chrome components live in a new `src/lib/chrome/` directory rather
than joining `src/lib/shell/`. The distinction is ownership: `shell/` holds the generic region
machinery F000 built and later features compose into, while `chrome/` holds surfaces whose
shape is dictated by the prototype and which the fidelity gate judges. Keeping them apart
makes the gate's scope legible — everything under `chrome/` is measured against the baseline.

`tools/gate-fidelity/` matches the path the repository's existing ignore rules already
anticipate, including the note that its reference baseline is a golden file rather than a
build artifact.

## Constitution Re-Check (post-design)

Re-evaluated after Phase 1 and Phase 2. Constitution v1.2.1. No verdict regressed.

| Principle | Verdict | What the design added or changed |
|-----------|---------|----------------------------------|
| I. Design Fidelity | PASS | Strengthened materially. The prototype's layout dimensions are generated at build time rather than transcribed, so a value can only change by changing the prototype. The gate adds what token linting cannot see: a surface of the wrong size. |
| II. One Source of Truth | PASS | Reinforced. The one place a second source could have formed — hand-copied dimensions — is closed by generation. Phase 1 Reconciliation records where the destination list is authored by hand and what keeps it honest. |
| III. Decisions Recorded | PASS | Six decisions in `research.md`, each with rationale and rejected alternatives. The session migration policy is flagged for Appendix A promotion; it governs every future schema change, not just this one. |
| IV. Open Items Block | PASS | No open item falls in scope. `[OPEN: OBS]` is respected rather than worked around: the gate writes a human-readable verdict, and no metric names were invented. |
| V. Interaction Budget | PASS | SC-008 bounds destination switching at 100 ms. The interface renders selection from local state and the write stays on the existing debounced path, so persistence cannot enter the gesture. |
| VI. Trust Boundaries | PASS | One added command surface, validated at the boundary like the rest. Destination identifiers arrive as strings and are matched against the catalogue rather than trusted. |
| VII. Tests Ship With Features | PASS | Unit for ordering and collapse rules; integration for the migration, which is the riskiest change here; end-to-end for navigation, keyboard reachability and greyscale. The fidelity gate is a fourth kind of check rather than a substitute for any of them. Linux-only end-to-end is the standing project limitation, cited rather than re-argued. |
| VIII. Ports and Adapters | PASS | No new outbound port, which was the expected and desirable outcome. The build-time gate sits outside the application boundary entirely and is not a component of it. |

**Design changes driven by the re-check**: none required. Four discrepancies between the
architecture and the Phase 1 artifacts were found during Phase 2 and are recorded in the
Phase 1 Reconciliation table in `architecture.md` rather than here.

**One risk worth naming outside the gate table**: the session migration is the most dangerous
change in this feature. Every other defect here is cosmetic; getting migration wrong silently
destroys a user's layout on upgrade. It has a dedicated integration test, and the quickstart
carries a scenario that exercises it against a real older file.

## Complexity Tracking

> No constitution violations. The end-to-end platform limitation this feature inherits is
> already justified project-wide in Appendix A, A-E2E, and needs no separate entry.
