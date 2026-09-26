# Implementation Plan: Editor Integration

**Branch**: `feature/F006-editor-integration` | **Date**: 2026-09-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/008-editor-integration/spec.md`

---

## Summary

Bind Monaco to the workspace cache so a file opens and edits locally with nothing in the
keystroke path, add the engine's missing write path so a save can be refused rather than
overwrite, stream large files by range as they are scrolled, and give the tabs F000 already
restores something behind them.

Four of the five pieces are additions to seams that already exist. `read_file` with ranges is
F003's and works. The `WorkspaceProvider` port already declares `write_file` and refuses it with
`Owner::F006Editor`. `workspace/onFileEvent` already reaches the interface, by the route F010
built. The one genuinely new surface is `workspace/writeFile` in the engine, which today answers
`METHOD_NOT_FOUND`.

---

## Technical Context

**Language/Version**: Rust 1.75 (workspace MSRV) for `engine` and `client/core`; TypeScript 5
with Svelte 5 for the webview. Unchanged by this feature.

**Primary Dependencies**: `monaco-editor` — the one addition, and confined to the webview.
Existing: `serde`/`serde_json` (protocol), `sha2` (hashing, already used by the cache),
`rusqlite` (cache), `tokio` (client only; the engine stays synchronous and runtime-free).

**Storage**: the SQLite workspace cache F003 defines (§5.2) for content and hashes; the JSON
session store A-STATE defines for open tabs, focus and — new here — the autosave preference.

**Testing**: `cargo test` for engine and client core; `vitest` for webview units; WebdriverIO
for end-to-end against the real window, with the local engine mode F010 added.

**Target Platform**: Tauri desktop client on macOS and Linux; `ide-engine` on Linux.

**Project Type**: desktop application — thin client, remote engine, one SSH connection.

**Performance Goals**: §1.4's budget. Keystroke to glyph is **0 ms network**, which this feature
measures as a request count rather than a duration. First visible window of a large file at p99
under 250 ms.

**Constraints**: 1 MiB frame cap (§4.1); the engine is synchronous and runtime-free; Monaco and
Svelte types may not cross into `client/core` (Principle VIII); Monaco's own language web workers
are disabled (FR-004).

**Scale/Scope**: 32 functional requirements, 15 success criteria, 4 user stories. One new engine
method, one new client provider method, one new webview surface.

### Fixed Quantities

Requirements that say "a stated quantity" are fixed here, with the reasoning, because a number
chosen during implementation is a number nobody reviewed.

| Quantity | Value | Why this value |
|---|---|---|
| Chunk threshold (FR-016, FR-018) | **1 MiB** | The frame cap of §4.1. A file that fits in one frame costs one round trip whole; ranging it costs the same round trip and delivers less. Below the cap, ranging is strictly worse. |
| Range size for scrolled reads (FR-017) | **256 KiB** | Four ranges per frame's worth, so a stall costs a quarter of the worst case, and well clear of the cap once the response is framed. A larger range risks the cap; a smaller one multiplies round trips across a long file. |
| Autosave debounce (FR-007c) | **2 s** after typing stops | §1.5's rule 2 debounces completion at 50–100 ms because the developer is waiting for the answer. Nobody waits for a save, and a save is a write with conflict consequences, so it is bound by how long a developer tolerates their work being unwritten rather than by perceived latency. Two seconds is short enough that a crash loses a sentence, long enough that ordinary typing produces one write per pause rather than per word. |
| Maximum file opened as text | **64 MiB** | Monaco holds the whole model in memory once loaded, and beyond this the editor is not the right surface. Refusing with a reason beats a window that stops responding. |
| Editor syntax colours extracted | **5** | What the prototype's editor actually distinguishes: punctuation, name, keyword, type, comment. See research.md, *The editor's palette is five colours and a deferral*. |

---

## Constitution Check

Evaluated against `.specify/memory/constitution.md` before Phase 0.

| Principle | Verdict | Basis |
|---|---|---|
| **I. Design Fidelity** | **PASS, with work** | The prototype has an `Editor` screen whose code colours are **raw hex** — `#9397ab`, `#e4e7f5`, `#b5abfc`, `#d2cefd`, `#75798c` — none of them design-system tokens, exactly as the terminal's hues were before A-TERMPALETTE. They are extracted by `ds-sync` rather than transcribed, and Monaco is themed from the resulting tokens; a full syntax theme is logged as owed to the design system. `--vk-code` (13.5px) and `--vk-line` (25px) already exist. No deviation is planned, so no designer approval is required. |
| **II. One Source of Truth** | **PASS** | Every value traces to `project-apex-predator.md`: the method and its fields to §4.8, `-32004` to §4.4, the budget to §1.4, ranged reads to §4.6, the provider signature to §6.1. Where this plan fixes a number the system leaves open, *Fixed Quantities* says so. |
| **III. Decisions Recorded First** | **PASS, with an obligation** | Two decisions here close genuine alternatives and are recorded in Appendix A before implementation: the editor palette and its deferral, and how a write is told apart from a foreign change. Recorded at this checkpoint, not after. |
| **IV. Open Items Block** | **PASS** | Zero live `[OPEN:]` markers before Appendix A, verified by injecting one and confirming the detector fires. |
| **V. Interaction Budget Verified** | **PASS** | SC-001 and SC-002 are request counts, SC-005 is a printed p99 over ≥100 samples per A-NFR. The absolute rule — a keystroke renders without awaiting the network — is the feature's first requirement and its first test. |
| **VI. Trust Boundaries Both Sides** | **PASS, and this is the riskiest part** | `writeFile` is the first method that **writes** the developer's filesystem with their full rights. The engine canonicalises and asserts containment independently of the client, rejects symlinks escaping the root, and bounds the content it will accept. See research.md, *Writing is the first method that can destroy something*. |
| **VII. Tests At Every Level** | **PASS** | Unit: hash comparison, threshold arithmetic, range stitching, the ending of a conflict. Integration: `writeFile` against a real filesystem, including the refusal and the containment checks. End to end: each acceptance scenario, against the real window with a real engine. No level is omitted. |
| **VIII. Ports and Adapters** | **PASS** | `write_file` already exists as a port method. Monaco lives in the webview and its types never reach `client/core`. The engine gains an inbound dispatch arm and a use case; the filesystem is already a port. |

**Gate result before Phase 0: PASS.** No violations, no justifications required in *Complexity
Tracking*.

### Re-check after Phase 2 design

Re-evaluated against the design as written, not against the intention.

| Principle | Verdict | What changed or was confirmed |
|---|---|---|
| **I. Design Fidelity** | **PASS** | Five colours extracted by `ds-sync`, none written by a component, Monaco themed from tokens. Unchanged by the design. |
| **II. One Source of Truth** | **PASS** | The contract defers to §4.8 explicitly and says so in its opening line. |
| **III. Decisions Recorded First** | **PASS** | Two records promoted to Appendix A at this checkpoint, before any code: the editor palette and the write-echo rule. |
| **IV. Open Items Block** | **PASS** | Unchanged. |
| **V. Interaction Budget** | **PASS, and strengthened** | The design puts the typing path in one sequence diagram with a note that it contains no sink, no IPC and no engine — so a future reader adding a call there sees what they are breaking. |
| **VI. Trust Boundaries** | **PASS** | The design states both ends check and that the engine's checks are the ones the integration tests exercise. |
| **VII. Tests At Every Level** | **PASS** | Unit, integration and end-to-end all have named surface; quickstart §5 adds seven mutation checks, two of which exist because they are each other's failure mode. |
| **VIII. Ports and Adapters** | **PASS, with one thing named rather than glossed** | `BufferSet` is a module-level singleton in the webview. Principle VIII forbids hidden global singletons — and exempts the webview in the same breath: "Svelte components are inbound adapters; the component tree is not itself hexagonal and MUST NOT be forced into that shape." The prohibition is about wiring in the Rust binaries, where a composition root exists to be bypassed. It is recorded here because the phrase does apply on its face, and a reviewer should see that it was read rather than skipped. The Rust side adds one port method and one use case, both wired at the existing composition root. |

**Gate result after Phase 2: PASS.** One design change was made during Phase 1 reconciliation —
a save no longer fetches missing ranges — and it is recorded in architecture.md's *Phase 1
Reconciliation* rather than silently applied.

---

## Project Structure

### Documentation (this feature)

```
specs/008-editor-integration/
├── spec.md              # what and why
├── plan.md              # this file
├── research.md          # Phase 0 — decisions and rejected alternatives
├── data-model.md        # Phase 1 — entities and their rules
├── contracts/
│   └── write-file.md    # Phase 1 — the one new protocol method, and its guarantees
├── quickstart.md        # Phase 1 — how to prove it works, with measurements
├── architecture.md      # Phase 2 — components and boundaries
├── design.md            # Phase 2 — classes, interfaces, sequences
└── checklists/
    └── requirements.md  # spec quality gate
```

### Source Code (repository root)

```
engine/src/
├── adapters/inbound/rpc.rs            # + workspace/writeFile dispatch arm
├── adapters/outbound/std_fs.rs        # + write with containment and atomicity
├── application/ports/file_system.rs   # + write capability on the existing port
└── application/use_cases/workspace.rs # + the compare-then-write rule

client/core/src/
├── adapters/outbound/remote_workspace.rs  # + write_file, replacing the refusal
├── adapters/inbound/tauri_commands.rs     # + file_write, file_read_range
└── application/use_cases/                 # + the buffer's base-hash rules

client/ui/lib/editor/                      # new: the webview surface
├── EditorPanel.svelte                     # inbound adapter
├── model.svelte.ts                        # buffers, bases, dirty state
├── palette.ts                             # design tokens -> Monaco theme
├── ranges.ts                              # which byte ranges are loaded
└── sink.ts                                # the seam to the core

tests/
├── unit/                    # vitest: ranges, palette, buffer rules
└── e2e/                     # WebdriverIO: the acceptance scenarios
```

**Structure Decision**: the existing shape, extended. No new crate, no new binary. The webview
gains one directory beside `terminal/`, which is the closest precedent — it too is a Svelte
surface wrapping a third-party view over a byte stream, and it establishes where the palette,
the model and the IPC seam each belong.

---

## Complexity Tracking

No constitutional violations to justify.

One deliberate deferral, recorded so it is not mistaken for an oversight: Monaco's theme covers
five syntax roles, the number the prototype distinguishes, and the remainder of its token set
falls back to the editor's foreground colour. A complete theme is the design system's to define,
on the same terms A-TERMPALETTE set for the terminal's sixteen-colour ramp.
