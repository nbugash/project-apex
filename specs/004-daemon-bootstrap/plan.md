# Implementation Plan: Daemon Bootstrap

**Branch**: `feature/F002-daemon-bootstrap` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-daemon-bootstrap/spec.md`

## Summary

Get a working engine onto a remote host and establish a compatible session with it.

The feature has a consequence the specification does not state, because it is a planning
question rather than a requirement: **this feature creates the engine.** A handshake needs a
responder, and F001's mock is forbidden from implementing any §4.8 method — a test asserts that
no method name from the catalogue appears in its directory, which is what keeps it a framing
double rather than a second engine that would later drift from the real one. So `auth/handshake`
cannot be answered by the mock, and F002 delivers the first real `ide-engine` binary alongside
the client-side bootstrap that deploys it.

The engine built here is deliberately minimal: it answers the handshake, owns session identity,
and reports its own restarts. It implements no workspace method; F003 adds those.

The second planning insight is that **deployment does not use the JSON-RPC channel at all.**
At the moment an engine is being deployed there is no engine running, so there is no control
channel — and a multi-megabyte binary would not fit through one anyway, since §4.1 caps a frame
at 1 MiB. Deployment runs over the SSH connection F001 already holds, through the control master
that A-B1 established, which is precisely the "bulk transfer is cheap" property that decision
was bought for.

## Technical Context

**Language/Version**: Rust 1.75+ (edition 2021) for both the client core and the engine. The
interface layer is unchanged Svelte 5 and TypeScript 5.x, touched only for deployment progress.

**Primary Dependencies**: Existing — `tokio`, `serde`, `serde_json`, `thiserror`. New — a
cryptographic digest for artifact verification (selected in research.md). No SSH library: A-B1
stands, and deployment uses the system `ssh` client over F001's control master.

**Build ordering**: The engine is built before the client, sequenced by the existing script layer
rather than by Cargo. Cargo cannot depend on another crate's binary artifact on stable, and a
build script that invokes Cargo recursively races the outer invocation's lock. See research.md,
"How the client gets an engine binary to embed".

**Storage**: None new on the client. The engine holds session state in memory only, which is
what makes a session not survive an engine crash — a deliberate limit recorded in the spec's
clarifications.

**Testing**: `cargo test`. Integration tests spawn the real engine binary as a local child
process, exactly as F001's tests spawn the mock daemon, so the suite needs no network and no
remote host. The opt-in `sshd` suite from F001 gains the deployment path, which is the one thing
no local spawn can prove.

**Target Platform**: Client on Linux and macOS desktop. Engine on Linux x86-64 and ARM64, which
is what A-EC2's provisioned instances run.

**Project Type**: Desktop application plus a remote daemon. This feature is the first to produce
two shipped binaries that must agree with each other.

**Performance Goals**: First connect to a host with no engine present within 30 seconds on a
10 Mbit/s link, excluding instance wake (SC-002). Deployment progress reported at least once per
second (SC-013). Handshake adds no measurable cost to an established session.

**Constraints**: No elevated privileges anywhere (FR-005). No outbound internet access required
on the remote host. Nothing partially transferred may ever be executable (FR-006). The previous
engine stays intact until the replacement completes a handshake (FR-021b). Every behaviour
verifiable with no remote host, no network and no real engine beyond a locally spawned one
(FR-026, SC-010).

**Screenshot convention**: End-to-end screenshots are written to
`reports/screenshots/${OS}/${FEATURE}/`, where `FEATURE` is the **feature map identity** — `F002`
here — and not the spec directory number. The two diverge (F001's directory is
`003-ssh-transport-core`), and the map identity is the one the map guarantees never to renumber
or reuse. The segment is derived from the git branch, so a run on a feature branch files its own
screenshots with nothing to tag. This is a project convention rather than a requirement of this
feature, recorded here so the tasks that implement it trace to something.

**Scale/Scope**: One engine per host, one session per engine, one client attached at a time.
Deployment is measured in tens of megabytes; sessions in single digits.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|---|---|---|
| **I. Design Fidelity** | Applies | Deployment progress is new interface surface. It must be built from the design system's tokens and pass the adherence lint, like every other surface. No new visual language. |
| **II. One Source of Truth** | Pass | Normative values come from §3.8, §4.8, §15.3 and A-BOOT. This plan cites them; it does not restate them. The spec's "On the source of values" section names each. |
| **III. Decisions Recorded** | Pass | Phase 0 records every decision in research.md before implementation. Two are expected to bind later features and are marked for promotion to Appendix A. |
| **IV. Open Items Block** | **Pass** | No `[OPEN:]` marker remains anywhere in the system specification. H-BOOT, which defined most of this feature, was resolved as A-BOOT on 2026-09-23 — this feature could not have been planned before that. |
| **V. Interaction Budget Verified** | Applies | Deployment is not on the keystroke path, but it has its own measurable target (SC-002) and a liveness target (SC-013). Both are measured, not asserted, per A-NFR: p99, at the boundary, with harness delay excluded. |
| **VI. Trust Boundaries Both Sides** | Applies, partially | The engine exists from this feature onward, so its side of the boundary starts to matter. It has no workspace methods yet, so the path canonicalisation the principle names has nothing to canonicalise — that obligation lands on F003 and is recorded here so it is not lost. What does apply now: the client treats every frame the engine sends as untrusted, exactly as F001 already does, and the engine validates the handshake it receives rather than assuming a well-formed client. The spec's threat model section covers why supply integrity is a separate axis and does not weaken this. |
| **VII. Every Feature Ships With Tests** | Applies | Four levels per A-TEST. End-to-end remains Linux-only per A-E2E, cited rather than re-argued. |
| **VIII. Ports and Adapters** | Applies | Bootstrap policy — when to deploy, what the version comparison means, when to promote a replacement — is a use case behind ports. The transfer mechanism is an adapter, which is what lets the whole policy be tested without moving a byte. |

**Gate result: PASS.** No violations to justify; Complexity Tracking is empty below.

### Re-evaluated after Phase 2 design

The design changed two things the pre-check could not have anticipated. Both were checked
against the constitution rather than assumed to be fine.

**The framing codec moves into a shared `protocol` crate.** F001's module documentation states
that "nothing outside this directory may depend on the codec", and this appears to contradict it.
It does not, and the distinction is worth stating because the next person will hit it too: that
rule keeps the *application layer* from reaching around its port, which still holds — the
client's use cases see `RequestTransport` and nothing below it. Sharing the wire format with the
process at the other end of the wire is not a layering violation; it is what a protocol is. The
alternative is two implementations of a format §4.1 defines exactly, which is precisely the
drift the mock daemon exists to prevent. Principle VIII is satisfied; research.md records the
reasoning.

**The protocol gains `session/onRestart`.** This is an edit to §4.8, which Principle II makes
the source of truth. The edit is part of implementing this feature and must land in
`project-apex-predator.md` — not be described only here, which would make this plan a second
source for a method signature. Recorded as an implementation obligation so `/speckit-tasks`
carries it rather than discovering it.

**Principle VI now binds a second process.** The engine exists from this feature onward and
validates the handshake it receives rather than assuming a well-formed client. It still has no
workspace method, so the path canonicalisation the principle names has nothing to canonicalise;
that obligation lands on F003 and is recorded in the pre-check above so it is not lost between
features.

No new violations. Gate still **PASS**.

## Project Structure

### Documentation (this feature)

```text
specs/004-daemon-bootstrap/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
├── architecture.md      # Phase 2 output
├── design.md            # Phase 2 output
└── tasks.md             # Phase 3 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src-tauri/                          # The client, as F000/F001 left it
├── src/
│   ├── domain/
│   │   ├── artifact.rs             # NEW — EngineArtifact, Digest, Architecture
│   │   └── session.rs              # EXISTING (persisted session) — untouched
│   ├── application/
│   │   ├── ports/
│   │   │   ├── deployer.rs         # NEW — ArtifactDeployer: stage, verify, promote
│   │   │   └── handshake.rs        # NEW — HandshakePeer
│   │   └── use_cases/
│   │       └── bootstrap.rs        # NEW — the policy: detect, deploy, handshake, decide
│   └── adapters/outbound/
│       └── sftp_deploy/            # NEW — transfer over F001's control master
│           └── mod.rs
└── tests/
    ├── bootstrap_deploy.rs         # NEW — US1
    ├── bootstrap_handshake.rs      # NEW — US2, US3
    └── bootstrap_restart.rs        # NEW — US4, US5

engine/                             # NEW CRATE — the first real ide-engine
├── Cargo.toml
└── src/
    ├── main.rs                     # stdio framing loop, reusing F001's codec
    ├── session.rs                  # session identity and lifetime
    └── handshake.rs                # auth/handshake, capability advertisement

src/lib/                            # Interface, touched only for progress
└── statusbar/
    └── presentation.ts             # EXTENDED — a deploying state
```

**Structure Decision**: The client keeps the hexagonal layout F000 established and F001
extended, with bootstrap policy as a use case and transfer as an adapter behind a port. The
engine becomes a **separate crate in a Cargo workspace**, not a module of the client, because it
is a separately deployed artifact that runs on a different machine and must be buildable for
targets the client is never built for. Sharing the framing codec between them is a deliberate
consequence of that split and is addressed in research.md rather than solved by copying it.

## Complexity Tracking

> No Constitution Check violations. This table is intentionally empty.
