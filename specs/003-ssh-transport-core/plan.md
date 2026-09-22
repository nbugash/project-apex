# Implementation Plan: SSH Transport Core

**Branch**: `feature/F001-ssh-transport-core` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-ssh-transport-core/spec.md`

## Summary

Build the link every remote feature rides on: spawn the system OpenSSH binary as a child
process, hold one authenticated master connection for the session, and speak
length-prefixed JSON-RPC over that child's stdio. A correlation registry matches each reply
to the request that asked for it; a priority-ordered send path keeps interactive traffic
ahead of background work; a supervisor re-establishes the connection when it drops.

The feature is verifiable before any remote engine exists. A mock daemon speaks the framing
and nothing above it, and can be driven into every failure condition and across a link with
250 ms round-trip time and 5% packet loss. That is what makes this feature finishable while
`[OPEN: H-BOOT]` remains open.

This replaces `StubConnectionStatusSource`, the F000 placeholder, and is the first feature to
exercise the port structure Principle VIII exists to buy: the swap is one binding in
`composition.rs`.

## Technical Context

**Language/Version**: Rust 1.75+ (core, edition 2021). No interface-layer work: this feature
adds no surface the webview renders beyond the connection state F000 already displays.

**Primary Dependencies**: `tokio` (already present — process spawn, async I/O, `oneshot` for
correlation, `time` for timeouts and backoff); `serde`/`serde_json` (already present — JSON-RPC
bodies). The system `ssh` binary is a runtime dependency, not a linked one (A-B1). **No new
crate is added for SSH**, which is the point of A-B1.

**Storage**: N/A — the transport holds no durable state. Connection state is in memory and
dies with the process. Credentials are the agent's and OpenSSH's business; this feature never
stores one.

**Testing**: `cargo test` for unit and integration. The mock daemon is a test-only binary
target in the same crate, driven over a pipe, so the transport under test is the real one.
No network and no remote host (SC-010).

**Target Platform**: macOS 13+ and Linux (x86_64, aarch64), matching F000. The assisted
authentication path needs OpenSSH 8.4+ and degrades on older platforms (§3.3).

**Project Type**: desktop-app — Rust core only for this feature.

**Performance Goals**: Transport adds ≤ 15 ms at the 99th percentile over the link's own
round trip (SC-011). Connection loss detected within the keepalive window, ~45 s (SC-009).
Interactive traffic ordered ahead of background work (SC-013).

**Constraints**: One authentication per session (SC-002), no process outliving the app
(SC-003), no credential in any log (FR-008), frames capped at 1 MiB (FR-010), registry
retains nothing for resolved requests (SC-005), failure classification stable across system
languages (SC-008).

**Scale/Scope**: One connection per session. Concurrent in-flight requests bounded by the
editor's own behaviour — hundreds, not millions. Frames are small by construction; anything
large belongs on SFTP (§3.6), which this feature does not build.

## Constitution Check

_GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design._

Evaluated against constitution v1.2.1.

| Principle                     | Verdict                   | Basis                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ----------------------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| I. Design Fidelity            | **PASS — not engaged**    | This feature renders nothing. The connection state it produces is displayed by the status bar F000 already built to the prototype's metrics. No new surface, so no fidelity surface.                                                                                                                                                                                                                                                 |
| II. One Source of Truth       | **PASS**                  | The transport decision is A-B1 and §3.1; this plan references them and restates neither. The one place duplication threatens is the framing and error-code tables in §4 — recorded in research.md as a deliberate single-source decision, not copied.                                                                                                                                                                                |
| III. Decisions Recorded       | **PASS**                  | Four decisions were recorded in the spec's Clarifications before this plan existed. Feature-local technical decisions are recorded in `research.md`. Two meet the Appendix A bar and are marked for promotion — see "Gate note" below.                                                                                                                                                                                               |
| IV. Open Items Block          | **PASS with a boundary**  | `[OPEN: H-BOOT]` sits in §3.8 and is the one open item that touches this feature's edge. It is excluded by scope: this feature classifies a missing engine and hands off (FR-017), and F002 owns installation. `[OPEN: NFR]` is engaged and handled — SC-011 measures what this feature alone can be held to, recorded in Assumptions. `[OPEN: OBS]` is respected rather than worked around: logging only, no invented metric names. |
| V. Interaction Budget         | **PASS with obligations** | This is the feature the budget is _about_. SC-011 and SC-013 are its measurable form, and both must fail the build when breached rather than be asserted. The mock's latency and loss simulation is what makes them measurable at all.                                                                                                                                                                                               |
| VI. Trust Boundaries          | **PASS with obligations** | Two boundaries, both real. The child process's stdout is untrusted input: a malformed or hostile frame must not desynchronise the stream or allocate unboundedly (FR-010). And the passphrase crosses a process boundary through a pipe — never logged, buffer cleared (FR-008).                                                                                                                                                     |
| VII. Tests Ship With Features | **PASS**                  | Unit, integration and the mock-driven suite are all satisfiable with no network. This feature has no end-to-end webview surface, so the macOS end-to-end gap recorded for F000 does not apply here.                                                                                                                                                                                                                                  |
| VIII. Ports and Adapters      | **PASS**                  | Ports named below. This feature is the first real test of the structure: it replaces an adapter F000 wrote specifically to be replaced.                                                                                                                                                                                                                                                                                              |

### Ports introduced or fulfilled by this feature

| Port                     | Direction | Purpose                                                                      | Adapter(s)                                                                                             |
| ------------------------ | --------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `ConnectionStatusSource` | outbound  | **Existing** (F000). Now fulfilled by the real transport instead of the stub | `SshTransport` replaces `StubConnectionStatusSource`                                                   |
| `RequestTransport`       | outbound  | Send a request, get a reply or a failure; withdraw a request                 | `SshTransport` (real), `MockTransport` (tests)                                                         |
| `CredentialPrompt`       | inbound   | Ask the interface layer for a passphrase and receive it                      | `TauriCredentialPrompt`; the askpass helper is a separate binary that reaches this through local IPC   |
| `ProcessSpawner`         | outbound  | Start and supervise a child process                                          | `OpenSshSpawner` (real), `ScriptedSpawner` (tests, drives failure classification without a real `ssh`) |

`ProcessSpawner` is what lets failure classification (User Story 4) be tested exhaustively.
Driving a real `ssh` into six distinct failure modes reliably is not practical; driving a
scripted spawner that emits chosen exit codes and stderr is.

### Gate note on Principle III

Two decisions in `research.md` bind features beyond this one and should be promoted to
Appendix A: the reconnection policy (because every feature that issues requests must know
what happens to an in-flight request when the link drops), and the priority classification of
outbound traffic (because every future caller must know which class its traffic is in).

Flagged here rather than acted on; amending the system specification is not this command's
scope.

## Project Structure

### Documentation (this feature)

```text
specs/003-ssh-transport-core/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── transport.md     # The RequestTransport port's contract
│   └── framing.md       # Wire format conformance, referencing §4.1
├── architecture.md      # Phase 2 output
├── design.md            # Phase 2 output
├── checklists/
│   └── requirements.md  # Spec quality checklist
└── tasks.md             # Phase 3 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src-tauri/
├── src/
│   ├── domain/
│   │   ├── connection.rs           # EXISTING — ConnectionState gains retrying/given-up
│   │   ├── request.rs              # NEW — RequestId, Priority, outcome, time limit
│   │   └── failure.rs              # NEW — FailureCondition and its classification rules
│   ├── application/
│   │   ├── ports/
│   │   │   ├── connection.rs       # EXISTING — ConnectionStatusSource
│   │   │   ├── transport.rs        # NEW — RequestTransport
│   │   │   ├── credential.rs       # NEW — CredentialPrompt
│   │   │   └── spawner.rs          # NEW — ProcessSpawner
│   │   └── use_cases/
│   │       ├── connect.rs          # NEW — two-phase connect, identity fallback
│   │       ├── exchange.rs         # NEW — send, correlate, time out, withdraw
│   │       └── supervise.rs        # NEW — loss detection, backoff, re-establish
│   └── adapters/
│       └── outbound/
│           ├── stub_connection.rs  # EXISTING — retired from composition, kept for tests
│           ├── openssh/
│           │   ├── mod.rs          # SshTransport: the composition of the pieces below
│           │   ├── spawner.rs      # OpenSshSpawner — the §3.1 invocation, one place
│           │   ├── framing.rs      # Content-Length codec over the child's stdio
│           │   ├── registry.rs     # RequestId -> oneshot::Sender
│           │   ├── sendq.rs        # Priority-ordered outbound queue
│           │   └── classify.rs     # Exit code + locale-pinned stderr -> FailureCondition
│           └── askpass/
│               └── ipc.rs          # The local channel the askpass helper talks to
├── bin/
│   └── apex-askpass.rs             # NEW — the helper OpenSSH executes (SSH_ASKPASS)
├── tests/
│   ├── mock_daemon/                # The mock: framing only, scriptable failures, latency
│   ├── transport_exchange.rs       # Correlation, timeout, withdrawal, ordering
│   ├── transport_failures.rs       # Six classifications via ScriptedSpawner
│   └── transport_recovery.rs       # Drop, backoff, re-establish, outstanding requests
└── Cargo.toml
```

**Structure Decision**: The transport is one outbound adapter directory, not a crate. Its
internals (`framing`, `registry`, `sendq`, `classify`) are private modules because nothing
outside the adapter may depend on them — the application layer sees `RequestTransport` and
nothing else. That is what makes the mock a drop-in and what will make a future transport
change a one-directory change.

`apex-askpass` is a **separate binary target** because OpenSSH execs it as a process. It is
deliberately tiny: read a prompt, ask the running app over local IPC, write the answer to
stdout, zero the buffer. All judgement lives in the app.

## Constitution Re-Check (post-design)

Re-evaluated after Phase 1 and Phase 2. Constitution v1.2.1. No verdict regressed.

| Principle                     | Verdict            | What the design added or changed                                                                                                                                                                                                                                                                                                    |
| ----------------------------- | ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| I. Design Fidelity            | PASS — not engaged | Unchanged. The design adds no rendered surface.                                                                                                                                                                                                                                                                                     |
| II. One Source of Truth       | PASS               | Strengthened. `contracts/framing.md` deliberately does _not_ restate §4.1's format, and `research.md` records why. The one risk found during Phase 1 — restating the error-code table for reader convenience — was identified and rejected rather than absorbed.                                                                    |
| III. Decisions Recorded       | PASS               | Ten decisions in `research.md`, each with rationale and rejected alternatives. Two marked for promotion to Appendix A.                                                                                                                                                                                                              |
| IV. Open Items Block          | PASS               | Design surfaced no new dependency on an open item. `[OPEN: OBS]` is respected concretely: the architecture's Observability row is logging only, and the latency measurement SC-011 needs lives in the test harness rather than becoming invented production telemetry.                                                              |
| V. Interaction Budget         | PASS               | Strengthened and made measurable. SC-011 measures _added_ overhead, which the mock's frame-level delay makes isolable; SC-013's wording was reconciled with what the wire format can actually deliver — ordering between frames, not within one. A criterion promising interleaving would have been unmeetable and quietly ignored. |
| VI. Trust Boundaries          | PASS               | Strengthened. `contracts/framing.md` enumerates nine hostile or awkward inputs the reader must survive, including the three that are simply how pipes behave. `Secret` makes leaking a passphrase require deliberate effort rather than merely being avoided by care.                                                               |
| VII. Tests Ship With Features | PASS               | The `ProcessSpawner` port is what makes all seven failure classifications testable without a remote host, and the mock makes latency and loss reproducible. Both are deliverables, not scaffolding.                                                                                                                                 |
| VIII. Ports and Adapters      | PASS               | Four ports, each a genuine seam. The transport's internals are private modules so nothing outside the adapter can depend on them — which is what makes `MockTransport` a drop-in rather than a parallel implementation.                                                                                                             |

**Design changes driven by the re-check**: four, all recorded in the Phase 1 Reconciliation
table in `architecture.md` rather than here.

## Complexity Tracking

| Violation                               | Why Needed                                                                                                                                                                                                                           | Simpler Alternative Rejected Because                                                                                                                                                                                                               |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A second binary target (`apex-askpass`) | `SSH_ASKPASS` names a program that OpenSSH executes; it cannot be a function in our process. §3.3 makes it normative and the path must be absolute.                                                                                  | Skipping askpass entirely would mean either a terminal prompt — which an IDE must not do — or supporting only agent-held keys, which excludes every developer with a passphrase on their key.                                                      |
| Four ports for one feature              | Each is a genuine seam: the spawner makes six failure modes testable without a real `ssh`, the credential prompt crosses into the interface layer, the transport is what F002+ depend on, and the connection source already existed. | Fewer ports would mean testing failure classification against a real `ssh` binary driven into `Permission denied`, `REMOTE HOST IDENTIFICATION HAS CHANGED` and exit 127 on demand — unreliable, slow, and impossible in CI without a remote host. |
