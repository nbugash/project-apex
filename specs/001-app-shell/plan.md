# Implementation Plan: Application Shell

**Branch**: `001-app-shell` | **Date**: 2026-09-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-app-shell/spec.md`

## Summary

Build the desktop application frame: a single window with a dockable region layout, tab
management for open documents, a status area reporting workspace and connection state, and
the approved dark appearance applied from the first painted frame. The frame persists and
restores across restarts, and degrades to a working default when persisted state is unusable.

This feature has no network surface. Connection state is fed by a stub so the shell is
independently testable before the transport exists (F001). The webview-to-core bridge built
here is the inbound adapter every later feature enters through.

## Technical Context

**Language/Version**: Rust 1.75+ (core, edition 2021); TypeScript 5.x (interface layer)

**Primary Dependencies**: Tauri v2 (A-B7); Svelte 5 + Vite (interface); the signed-off
Nocturne design system stylesheet, Inter and JetBrains Mono webfonts, Phosphor icon font,
all bundled from `mockups/` (Constitution Principle I); `serde`/`serde_json` for state
persistence; `tokio` for background work off the interface thread

**Storage**: A single JSON state file in the platform application-data directory. Not SQLite
— see [research.md](./research.md), "Layout persistence mechanism"

**Testing**: `cargo test` (Rust unit and integration); Vitest (interface-layer unit);
`tauri-driver` + WebdriverIO for end-to-end on Linux; scripted smoke check on macOS — see
[research.md](./research.md), "End-to-end testing on macOS"

**Target Platform**: macOS 13+ and Linux (x86_64 and aarch64). Windows is out of scope per
spec §1.3

**Project Type**: desktop-app

**Performance Goals**: Launch to interactive window under 2 s on reference hardware (SC-001);
no interaction stall exceeding 100 ms (SC-004); layout drag and resize sustained at display
refresh rate

**Constraints**: Dark appearance only, no light variant (FR-015, FR-016); every visible
surface styled from design tokens, platform defaults are defects (FR-021); no light or
unstyled frame during launch (FR-020); keyboard operable (FR-018); least-privilege capability
set (FR-017); no network dependency

**Scale/Scope**: One window per running instance; three regions; tab strip sized to open
documents; single local user

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

Evaluated against constitution v1.2.1.

| Principle | Verdict | Basis |
|-----------|---------|-------|
| I. Design Fidelity | **PASS with obligations** | The feature consumes the signed-off stylesheet, fonts and icon set directly rather than reimplementing them. Obligations carried into tasks: token-only styling, no platform default focus ring, and the adherence lint running in CI. The lint ships configured for React and MUST be ported without weakening its rules — this is the outstanding conflict 3 recorded in the constitution, and this feature owns closing it. |
| II. One Source of Truth | **PASS** | The architecture contradiction that previously blocked planning is resolved and recorded as A-UI. No competing document remains. |
| III. Decisions Recorded | **PASS with a note** | Feature-local decisions are recorded in `research.md`. See "Gate note on Principle III" below. |
| IV. Open Items Block | **PASS** | No `[OPEN: …]` marker in the system specification falls inside this feature's scope. `SIGN` and `UPDATE` belong to F011, `H-BOOT` to F002, `EC2` to F004. `TEST` is constrained by Principle VII and resolved for this feature's scope in `research.md`. |
| V. Interaction Budget | **PASS with obligations** | SC-001 and SC-004 are the measurable form of the budget for this feature. A measurement that fails when either is breached is a deliverable, not a nicety. No keystroke path exists yet; the rule that matters here is that persistence writes never block interaction. |
| VI. Trust Boundaries | **PASS with obligations** | The webview-to-core bridge is a real trust boundary even with no network present: the interface layer renders content and the core holds filesystem rights. Command inputs are validated in the core regardless of interface-side checks. |
| VII. Tests Ship With Features | **PASS with a recorded limitation** | Unit and integration levels are fully satisfiable. End-to-end is satisfiable on Linux but not on macOS, where no WebDriver implementation exists for the platform webview. Recorded in Complexity Tracking with its justification. |
| VIII. Ports and Adapters | **PASS** | Ports named below; both directions are explicit and the composition root is a single module. |

### Ports introduced by this feature

Required by Principle VIII and by the Development Workflow gate.

| Port | Direction | Purpose | Adapter(s) in this feature |
|------|-----------|---------|-----------------------------|
| `SessionStore` | outbound | Load and persist window geometry, region layout, open document references and focus | `JsonFileSessionStore` (application-data directory) |
| `ConnectionStatusSource` | outbound | Supply the current connection state to the status area | `StubConnectionStatusSource` (replaced by the transport adapter in F001) |
| `ShellCommands` | inbound | The operations the interface layer can invoke on the core | `TauriCommandAdapter` |

`SessionStore` and `ConnectionStatusSource` are capabilities, not technologies, so the
JSON-backed implementation and the stub are both swappable without touching a use case. That
is the property F001 depends on: it replaces the stub adapter and nothing else changes.

### Gate note on Principle III

Principle III requires decisions that close a genuine alternative to be recorded in Appendix A
of the system specification. Read strictly, every stack choice in `research.md` would go
there, which would turn Appendix A into a per-feature changelog and defeat its purpose.

The working reading applied here: `research.md` is the decision record for decisions whose
blast radius is one feature, and Appendix A is for decisions that bind other features. Two
decisions in this feature's `research.md` meet the Appendix A bar and should be promoted:
session state living outside the workspace cache, and end-to-end coverage being asymmetric
across target platforms. Both constrain later features.

This distinction is not in the constitution text. It is flagged here rather than acted on,
because amending the constitution is not this command's scope.

## Project Structure

### Documentation (this feature)

```text
specs/001-app-shell/
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

```text
src-tauri/                          # Rust core
├── src/
│   ├── main.rs                     # Entry point; window creation
│   ├── composition.rs              # Composition root: the only place adapters are bound
│   ├── domain/
│   │   ├── layout.rs               # Layout, region sizing and minimum-size rules
│   │   ├── session.rs              # WorkspaceSession, open document ordering and focus
│   │   └── connection.rs           # ConnectionState
│   ├── application/
│   │   ├── ports/
│   │   │   ├── session_store.rs    # SessionStore (outbound)
│   │   │   └── connection.rs       # ConnectionStatusSource (outbound)
│   │   └── use_cases/
│   │       ├── restore_session.rs
│   │       ├── persist_session.rs
│   │       └── observe_connection.rs
│   └── adapters/
│       ├── inbound/
│       │   └── tauri_commands.rs   # ShellCommands inbound adapter
│       └── outbound/
│           ├── json_session_store.rs
│           └── stub_connection.rs
├── tests/
│   ├── session_store.rs            # Integration: real files, real corruption cases
│   └── display_geometry.rs         # Integration: window constrained to attached display
├── capabilities/                   # Tauri v2 permission scoping
└── tauri.conf.json

src/                                # Svelte interface layer (inbound adapters)
├── main.ts
├── lib/
│   ├── shell/
│   │   ├── Window.svelte           # Region composition
│   │   ├── Region.svelte           # One dockable region
│   │   └── Splitter.svelte         # Divider with minimum-size enforcement
│   ├── tabs/
│   │   ├── TabStrip.svelte
│   │   └── TabOverflow.svelte
│   ├── statusbar/
│   │   └── StatusBar.svelte
│   └── ds/                         # Design system, copied from mockups/ at build time
└── app.css                         # Imports the Nocturne stylesheet; defines nothing

tests/
├── unit/                           # Vitest: interface-layer logic
└── e2e/                            # WebdriverIO + tauri-driver (Linux CI)
```

**Structure Decision**: Two source trees in one repository, matching the two runtimes Tauri
produces: `src-tauri/` for the Rust core and `src/` for the interface layer. Inside the core,
the four-way split of `domain`, `application/ports`, `application/use_cases` and `adapters`
is the layout Principle VIII requires, with `composition.rs` as the single wiring point. The
interface layer is deliberately not organised hexagonally — Svelte components are inbound
adapters and Principle VIII explicitly exempts the component tree.

`src/lib/ds/` is populated from `mockups/` by the build rather than hand-authored, so the
design system cannot drift from the signed-off artifact.

## Constitution Re-Check (post-design)

Re-evaluated after Phase 1 and Phase 2. Constitution v1.2.1. No verdict regressed.

| Principle | Verdict | What the design added or changed |
|-----------|---------|----------------------------------|
| I. Design Fidelity | PASS | Strengthened. The design system is copied from `mockups/` by the build and `app.css` defines nothing of its own, so divergence requires editing the signed-off artifact — a visible, reviewable act rather than silent drift. `lint:ds` is a required check in `quickstart.md`. |
| II. One Source of Truth | PASS | Unchanged. A-UI remains the sole record of the rendering decision. |
| III. Decisions Recorded | PASS with the note above | Eight decisions recorded in `research.md`, each with rationale and rejected alternatives. Two are marked for promotion to Appendix A. |
| IV. Open Items Block | PASS | Design surfaced no new dependency on an open item. `[OPEN: OBS]` is respected rather than worked around: observability is logging only, and no metric names were invented to fill the section. |
| V. Interaction Budget | PASS | Strengthened. Persistence is debounced and runs off the interaction path, so disk latency cannot enter a gesture. `perf:budget` asserts SC-001 and SC-004 as a required check. |
| VI. Trust Boundaries | PASS | The command adapter validates every argument at the boundary, and `contracts/shell-commands.md` states input is untrusted regardless of interface-layer checks. Validation is in the core, not the webview. |
| VII. Tests Ship With Features | PASS with the recorded limitation | Unit, integration and end-to-end suites are named in `quickstart.md` with the commands to run them. The macOS end-to-end gap is unchanged and remains justified in Complexity Tracking. |
| VIII. Ports and Adapters | PASS | Realised concretely: two outbound ports each with one adapter, one inbound adapter, a single composition root, and a `domain`/`application`/`adapters` split. The F001 transport swap is a one-adapter change, which is the property the principle exists to buy. |

**Design changes driven by the re-check**: none were required. Four discrepancies between the
architecture and the Phase 1 artifacts were found and resolved during Phase 2; they are
recorded in the Phase 1 Reconciliation table in `architecture.md` rather than here.


## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|--------------------------------------|
| Principle VII: end-to-end tests do not run on macOS | `tauri-driver` supports the Linux and Windows webviews; no WebDriver implementation exists for WKWebView on macOS, so the level is not merely inconvenient but unavailable. The feature ships end-to-end coverage on Linux in CI plus a scripted launch-and-screenshot smoke check on macOS, and the shared interface layer means most end-to-end risk is platform-independent. | Dropping end-to-end entirely would leave the four user-story journeys unverified on any platform. Hand-driving macOS through AppleScript or accessibility APIs was rejected as a bespoke harness costing more than the coverage gap it closes, for a shell whose platform-specific surface is window geometry and appearance — both cheaper to assert through integration tests and the smoke check. |
