---
description: "Task list for F002 daemon-bootstrap"
---

# Tasks: Daemon Bootstrap

**Input**: Design documents from `/specs/004-daemon-bootstrap/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/), [architecture.md](./architecture.md),
[design.md](./design.md)

**Tests**: Included. Constitution Principle VII requires them, and A-TEST names the four levels.

**Organization**: By user story, so each is independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel — different files, no dependency on incomplete work
- **[Story]**: US1–US5, matching [spec.md](./spec.md)

## Path Conventions

Paths follow the Module & File Layout in [design.md](./design.md). The repository becomes a
Cargo workspace: `protocol/`, `engine/` and the existing `src-tauri/`.

**Screenshots** are written to `reports/screenshots/${OS}/${FEATURE}/`, where `FEATURE` is the
feature map identity — `F002` here, not the spec directory number. The two differ (F001's spec
directory is `003-ssh-transport-core`), and the map identity is the immutable one.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Turn one crate into a workspace and give the screenshots somewhere to go.

- [X] T001 Create the workspace root `Cargo.toml` listing `protocol`, `engine` and `src-tauri`, and verify `cargo test` at the root still runs F001's whole suite unchanged
- [X] T002 Create the `protocol` crate skeleton in `protocol/Cargo.toml` and `protocol/src/lib.rs` with no dependencies beyond `serde`
- [X] T003 Create the `engine` crate skeleton in `engine/Cargo.toml` and `engine/src/main.rs`, producing a binary that starts and exits cleanly
- [X] T004 [P] Derive the screenshot feature segment from the git branch in `tests/e2e/wdio.conf.ts` — `feature/F002-daemon-bootstrap` yields `F002` — with an `APEX_FEATURE` override and an explicit fallback when not on a feature branch. A run on a feature branch must file its own screenshots without anyone tagging anything
- [X] T005 [P] Update `capturedFiles()` in `tests/e2e/wdio.conf.ts` for the extra directory level. It currently reads exactly one level deep; with `${OS}/${FEATURE}/` it would count zero files and the capture gate would fail every run — or worse, pass while counting nothing if the comparison were loosened to fix it
- [X] T006 [P] Update the screenshot path convention in `tests/e2e/wdio.conf.ts` `afterTest` to `reports/screenshots/${OS}/${FEATURE}/`, and confirm the gate still fails when captures are missing by re-running the mutation check that proved it works

**Checkpoint**: Workspace builds, F001's suite passes unchanged, screenshots land under the new
convention and the gate still bites.

---

## Phase 2: Foundational (Blocking Prerequisites)

**⚠️ CRITICAL**: No user story work begins until this phase is complete.

- [X] T007 Move `FrameCodec` and its tests from `src-tauri/src/adapters/outbound/openssh/framing.rs` into `protocol/src/framing.rs` unchanged, and re-export it from the openssh adapter so F001's tests pass without edits
- [X] T008 Define the wire types in `protocol/src/wire.rs` per [data-model.md](./data-model.md): `HandshakeRequest`, `HandshakeResponse`, `RestartNotice`, `SessionId`, `CapabilitySet`, and the `PROTOCOL_VERSION` constant
- [X] T009 [P] Implement `EngineArtifact`, `Architecture` and `Digest` in `src-tauri/src/domain/artifact.rs` per [data-model.md](./data-model.md), including the rule that a digest is derived from bytes and never hand-written
- [X] T010 [P] Implement `DeploymentState` and `DeploymentFailure` in `src-tauri/src/domain/artifact.rs` with the six named failure causes
- [X] T011 [P] Define the `ArtifactDeployer` port in `src-tauri/src/application/ports/deployer.rs` with `deploy`, `retire_previous` and `observe`, matching the signatures in [design.md](./design.md)
- [X] T012 [P] Define the `HandshakePeer` port in `src-tauri/src/application/ports/handshake.rs`
- [X] T076 Sequence the engine build before the client build in the existing script layer (`package.json` and the CI workflow), producing the engine at a conventional path. Cargo cannot depend on another crate's binary artifact on stable, and a build script that invokes Cargo recursively races the outer invocation's lock — see research.md, "How the client gets an engine binary to embed"
- [X] T013 Extend `src-tauri/build.rs` to read the engine artifact from that path (overridable with `APEX_ENGINE_BIN`), embed its bytes and compute its SHA-256 at build time, so the constant and the bytes cannot disagree. A **missing** artifact must fail the build naming the skipped step — embedding an empty slice produces a client that ships, deploys zero bytes and fails verification against a host that did nothing wrong
- [X] T014 Add `session/onRestart` to the Session group in §4.8 of `project-apex-predator.md`, with its params and its notification kind. **This is a system specification edit, not a note in a plan** — Principle II makes §4.8 the source of truth for method signatures, and a method described only in `design.md` would make this feature a second source
- [X] T077 [P] Add an `assert_no_network()` helper to `src-tauri/tests/common/mod.rs` that walks this process's own descriptors against the kernel's TCP tables, and prove it works by opening a loopback socket and confirming the helper sees it. F001's equivalent lives in one test binary and checks only that process — each integration test file is a separate binary, so `bootstrap_*.rs` are entirely unchecked without this (FR-026, SC-010)
- [X] T015 [P] Implement `ScriptedDeployer` in `src-tauri/tests/common/mod.rs` — chosen failures, recorded calls, no bytes moved. This is the seam that makes every deployment failure path testable without a host

**Checkpoint**: Both crates compile, ports exist, the protocol is shared, and §4.8 describes the
notification the engine will send.

---

## Phase 3: User Story 1 - Connect to a machine that has never run the engine (Priority: P1) 🎯 MVP

**Goal**: A developer points the app at a bare host and reaches a working session with no manual
step.

**Independent Test**: Against a host with no engine, connect and confirm a session is established
and the deployed artifact matches what the client shipped.

### Tests for User Story 1

- [X] T016 [P] [US1] Failing test in `src-tauri/tests/bootstrap_deploy.rs`: a host reporting no engine triggers a deployment and reaches a session, with no developer action (SC-001)
- [X] T017 [P] [US1] Test in `src-tauri/tests/bootstrap_deploy.rs`: an artifact whose digest does not match is **never executed**. Assert on execution count being zero, not on the error returned — a deployment that ran the binary and then reported an error also returns an error (SC-004)
- [X] T018 [P] [US1] Test in `src-tauri/tests/bootstrap_deploy.rs`: an architecture with no embedded artifact is refused by name before anything transfers (FR-008)
- [X] T019 [P] [US1] Test in `src-tauri/tests/bootstrap_deploy.rs`: a second connect with a matching digest transfers nothing and still establishes a session (SC-003)
- [X] T020 [US1] Test in `src-tauri/tests/bootstrap_deploy.rs`: progress is published at least once per second while transferring, carrying bytes and total. Assert on the **number and spacing** of reports, not their existence — one report at the start satisfies "progress was reported" and still looks exactly like a hang (SC-013)
- [ ] T021 [US1] Test in `src-tauri/tests/bootstrap_deploy.rs`: interleaved concurrent deployments yield one valid engine or a reported failure, never a mixed artifact, across a sustained run rather than a single pair that may happen to serialise (SC-012)
- [X] T022 [P] [US1] Test in `src-tauri/tests/bootstrap_deploy.rs`: each of the six `DeploymentFailure` causes is reported as itself and not collapsed into a generic failure

- [ ] T078 [US1] Test in `src-tauri/tests/bootstrap_deploy.rs`: a deployment over a **simulated 10 Mbit/s link** completes within 30 seconds, and the measured value is printed rather than only compared (SC-002, A-NFR). Every other success criterion has a gate; a budget verified only against whatever link the developer happens to have is not one
- [X] T079 [P] [US1] Call `assert_no_network()` from `src-tauri/tests/bootstrap_deploy.rs` (SC-010)

### Implementation for User Story 1

- [X] T023 [US1] Implement `SshStreamDeployer::deploy` in `src-tauri/src/adapters/outbound/deploy/mod.rs`: stream to a digest-qualified staged path over F001's control master, counting bytes as they go
- [X] T024 [US1] Implement remote verification in `src-tauri/src/adapters/outbound/deploy/mod.rs` by invoking `sha256sum` on the host and comparing exactly — not by prefix, which is a weaker check that looks identical in a passing test
- [X] T025 [US1] Implement atomic promotion in `src-tauri/src/adapters/outbound/deploy/mod.rs`: set the executable bit only after verification, then `rename` within the same directory so the rename cannot silently become a copy across filesystems
- [X] T026 [US1] Implement progress publication in `src-tauri/src/adapters/outbound/deploy/mod.rs` on the cadence T020 asserts
- [X] T027 [US1] Implement artifact selection and the unsupported-architecture refusal — landed in `src-tauri/src/application/use_cases/bootstrap.rs` rather than `embedded.rs`: choosing which artifact suits a host is policy, and `embedded.rs` is storage. Placement corrected during implementation
- [X] T028 [US1] Implement the idempotence check in `src-tauri/src/adapters/outbound/deploy/mod.rs` — a present artifact with a matching digest transfers nothing
- [ ] T029 [US1] Implement `Bootstrap::establish` deployment path in `src-tauri/src/application/use_cases/bootstrap.rs`, consuming F001's `EngineMissing` classification, which today has no recipient
- [ ] T030 [P] [US1] Add the deploying state with progress to `src/lib/statusbar/presentation.ts` and its unit test in `tests/unit/status-bar.test.ts`, built from design tokens per Principle I

**Checkpoint**: A bare host reaches a session. This is the MVP.

---

## Phase 4: User Story 2 - Know what the engine can do before asking it (Priority: P1)

**Goal**: Versions and capabilities exchanged first; the client offers only what this engine
supports.

**Independent Test**: Handshake against engines advertising different capability sets and confirm
offered functionality follows, with no request for an unadvertised capability reaching the wire.

### Tests for User Story 2

- [X] T031 [P] [US2] Test in `src-tauri/tests/bootstrap_handshake.rs`: the handshake is the first request on a session, and nothing precedes it (FR-010)
- [X] T032 [P] [US2] Test in `src-tauri/tests/bootstrap_handshake.rs`: a request for a capability the engine did not advertise produces **no frame on the wire**. Assert on what was written, not on the error the caller received — a request that was sent and rejected also produces an error (SC-008)
- [X] T033 [P] [US2] Test in `src-tauri/tests/bootstrap_handshake.rs`: a handshake that is never answered fails distinguishably from a transport failure (FR-014)
- [X] T034 [P] [US2] Create `engine/src/handshake.rs` with its test module and a test that unknown capability tokens are ignored rather than rejected, on both sides — the property that lets a method be added without a version bump. This task creates the file; T036 fills in the responder, because a Rust unit test lives in the file it tests and cannot precede it

- [X] T080 [P] [US2] Test in `engine/src/handshake.rs`: a well-framed but malformed handshake payload is rejected without panicking and without the engine acting on any part of it. Constitution Principle VI is a MUST and makes inbound input untrusted at the receiving end; the codec tests inherited from `protocol` cover framing, not payloads, so nothing currently exercises this
- [X] T081 [P] [US2] Call `assert_no_network()` from `src-tauri/tests/bootstrap_handshake.rs` (SC-010)

### Implementation for User Story 2

- [X] T035 [US2] Implement the stdio frame loop in `engine/src/main.rs`, reusing `protocol::framing` and treating every inbound frame as untrusted per Principle VI
- [X] T036 [US2] Implement the `auth/handshake` responder in `engine/src/handshake.rs`, advertising the capability set this engine actually serves
- [X] T037 [US2] Implement `TransportHandshake` in `src-tauri/src/adapters/outbound/deploy/mod.rs` over F001's `RequestTransport`
- [X] T038 [US2] Record the engine's capabilities for the session's life in `src-tauri/src/application/use_cases/bootstrap.rs`
- [X] T039 [US2] Implement the local refusal for unadvertised capabilities in `src-tauri/src/application/use_cases/bootstrap.rs`, so the request never reaches the transport

**Checkpoint**: The client knows what it may ask for before it asks.

---

## Phase 5: User Story 3 - Refuse a protocol the client does not understand (Priority: P1)

**Goal**: A newer engine stops the session with a clear instruction, never a guess.

**Independent Test**: Drive the handshake with older, identical and newer protocol versions and
confirm redeploy, proceed and refuse respectively.

### Tests for User Story 3

- [X] T040 [P] [US3] Test in `src-tauri/tests/bootstrap_handshake.rs`: an engine reporting a newer protocol version refuses the session and names both versions (FR-017)
- [X] T041 [P] [US3] Test in `src-tauri/tests/bootstrap_handshake.rs`: **no request is ever exchanged with a newer engine** beyond the handshake itself (SC-005)
- [X] T042 [P] [US3] Test in `src-tauri/tests/bootstrap_handshake.rs`: the refusal has no override. Like F001's changed-host-key test, this asserts the absence of a path — a flag, option or retry that proceeds anyway must not exist (FR-018)
- [X] T043 [P] [US3] Test in `src-tauri/tests/bootstrap_handshake.rs`: an identical protocol version proceeds with no deployment (FR-019)
- [X] T044 [P] [US3] Test in `src-tauri/tests/bootstrap_handshake.rs`: an older protocol version triggers replacement without involving the developer (FR-016)

### Implementation for User Story 3

- [X] T045 [US3] Implement `VersionVerdict` in `src-tauri/src/application/use_cases/bootstrap.rs` as a comparison, never a negotiation
- [X] T046 [US3] Wire each verdict to its response in `src-tauri/src/application/use_cases/bootstrap.rs` per the table in [data-model.md](./data-model.md)
- [X] T047 [US3] Implement the `RefusedNewerEngine` outcome in `src-tauri/src/application/use_cases/bootstrap.rs`, carrying both versions so the message can name them

**Checkpoint**: Skew is safe in both directions.

---

## Phase 6: User Story 4 - Update the engine without the developer noticing (Priority: P2)

**Goal**: A newer client replaces an older engine and carries on, and a failed replacement leaves
a working one.

**Independent Test**: Connect with a client newer than the deployed engine; confirm replacement,
restart and resumed session with no developer action, and that a failed replacement is survivable.

### Tests for User Story 4

- [X] T048 [P] [US4] Test in `src-tauri/tests/bootstrap_restart.rs`: replacing an engine requires no reconnection by the developer (SC-006)
- [X] T049 [US4] Test in `src-tauri/tests/bootstrap_restart.rs`: a replacement that **verifies correctly and then fails to run** leaves the previous engine in place and serving. Write this before the corrupt-artifact case — verification passing is not proof of runnability, and a test that only corrupts the artifact never exercises this path (SC-007)
- [X] T050 [P] [US4] Test in `src-tauri/tests/bootstrap_restart.rs`: `retire_previous` is called only after a successful handshake, never on promotion (contracts/deployment.md)
- [X] T051 [P] [US4] Test in `src-tauri/tests/bootstrap_restart.rs`: a `retire_previous` failure is logged and the session continues — an orphaned binary costs disk, not correctness
- [X] T052 [P] [US4] Test in `src-tauri/tests/bootstrap_restart.rs`: an engine that starts and dies is redeployed at most three times before being reported as unable to run here (FR-022, SC-011)

### Implementation for User Story 4

- [X] T053 [US4] Implement version-qualified artifact paths in `src-tauri/src/adapters/outbound/deploy/mod.rs`, so the previous engine remains under its own name rather than being backed up
- [X] T054 [US4] Implement `retire_previous` in `src-tauri/src/adapters/outbound/deploy/mod.rs`, idempotent and non-fatal
- [X] T055 [US4] Implement re-execution in `engine/src/main.rs`, preserving the stdio file descriptors across `exec` so the channel survives
- [X] T056 [US4] Implement the replacement sequence in `src-tauri/src/application/use_cases/bootstrap.rs`: deploy, handshake, then retire — in that order, because only the use case sees both ports
- [X] T057 [US4] Implement the redeploy bound in `src-tauri/src/application/use_cases/bootstrap.rs`

**Checkpoint**: The estate can move forward without the developer participating.

---

## Phase 7: User Story 5 - Keep the session across an engine restart (Priority: P2)

**Goal**: The client knows a restart happened, what survived and what did not.

**Independent Test**: Force a restart with state registered against the session; confirm
notification, unchanged identity, and that unpreserved state is reported.

### Tests for User Story 5

- [X] T058 [P] [US5] Test in `src-tauri/tests/bootstrap_restart.rs`: a restart is announced by the engine and never inferred by the client (SC-009)
- [X] T059 [P] [US5] Test in `src-tauri/tests/bootstrap_restart.rs`: the session identity is unchanged across re-execution (FR-024)
- [X] T060 [P] [US5] Test in `src-tauri/tests/bootstrap_restart.rs`: state that did not survive is named in `unpreserved`, and an empty list is asserted to mean nothing was lost rather than nothing was checked (FR-025)
- [ ] T061 [US5] Test in `src-tauri/tests/bootstrap_restart.rs`: work in progress survives a disconnection and is still running when the client re-attaches (SC-009a)
- [X] T062 [US5] Test in `src-tauri/tests/bootstrap_restart.rs`: presenting an identity the engine has forgotten yields a stated refusal and a new session — never a silent new session presented as a resumption (FR-024c)
- [X] T063 [P] [US5] Test in `src-tauri/tests/bootstrap_restart.rs`: A-REQ still holds — an in-flight request dies with its connection even though the session outlives it. The two rules are easy to conflate and the distinction is the point

- [ ] T082 [P] [US5] Unit tests in `engine/src/session.rs`: mint yields distinct identities, resume of an unknown identity returns false, and identity is stable across re-execution. These are engine-side invariants currently covered only through the client's integration tests, which cannot fail for an engine-internal reason
- [X] T083 [P] [US5] Call `assert_no_network()` from `src-tauri/tests/bootstrap_restart.rs` (SC-010)

### Implementation for User Story 5

- [X] T064 [US5] Implement `SessionRegistry` in `engine/src/session.rs`: mint, resume, and the in-memory lifetime that makes a crash fatal to a session by design
- [X] T065 [US5] Implement resumption in `engine/src/handshake.rs`, setting `resumed` truthfully so the client can tell a new session from a re-attached one
- [X] T066 [US5] Implement `session/onRestart` emission in `engine/src/main.rs` after re-execution
- [X] T067 [US5] Implement restart handling in `src-tauri/src/application/use_cases/bootstrap.rs`, surfacing `unpreserved` rather than absorbing it
- [X] T068 [US5] Implement the refused-resumption path in `src-tauri/src/application/use_cases/bootstrap.rs`

**Checkpoint**: Every credential-free restart path reaches an outcome the developer can trust.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [X] T069 Add the opt-in real-`sshd` deployment test in `src-tauri/tests/bootstrap_real_sshd.rs`, skipped with a clear reason when unavailable. This is the only place the transfer, the remote `sha256sum` and the atomic promotion run against a real remote filesystem
- [X] T084 Assert in `src-tauri/tests/bootstrap_real_sshd.rs` that nothing in the deployment path requires elevated privilege: every path written is owned by the connecting account, and no `sudo`, `su` or setuid invocation appears in what the deployer runs (FR-005). This is the only suite with a real filesystem and a real account, so it is the only place the property is observable
- [ ] T070 [P] Document the engine and bootstrap in `docs/engine.md` — deployment, the handshake, the version rule, and what a session outlives
- [ ] T071 [P] Add `reports/screenshots/README.md` recording the `${OS}/${FEATURE}/` convention and why `FEATURE` is the map identity rather than the spec directory number
- [ ] T072 Run the full quickstart validation and record the results in [quickstart.md](./quickstart.md), including the SC-002 and SC-013 measurements as numbers rather than verdicts, per A-NFR
- [ ] T073 Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` across the workspace and fix what they report
- [ ] T074 Promote the two marked decisions in [research.md](./research.md) to Appendix A of `project-apex-predator.md` — that bulk data travels beside the protocol channel rather than through it, and that the protocol version increments only on breaking changes. Both bind every later feature, and a decision only F002's research records is one F003 will not find
- [X] T075 Verify by mutation that the capture gate in `tests/e2e/wdio.conf.ts` still fails when screenshots are missing, after the path convention change. The gate was proven once; a path change is exactly the kind of edit that silently unproves it

---

## Dependencies

```
Phase 1 Setup
    ↓
Phase 2 Foundational  ← blocks everything
    ↓
Phase 3 US1 (deploy)  ← MVP
    ↓
Phase 4 US2 (handshake) ← needs an engine to talk to, so needs US1's engine crate running
    ↓
Phase 5 US3 (version)   ← needs the handshake to report a version
    ↓
Phase 6 US4 (replace)   ← needs both deploy and handshake
    ↓
Phase 7 US5 (session)   ← needs re-execution from US4
    ↓
Phase 8 Polish
```

US1 and US2 are both P1 but not parallel: the handshake needs an engine, and US1 is what puts one
there. US3 depends on US2 for the same reason. US4 and US5 are genuinely sequential — a restart
notification needs something that restarts.

## Parallel Opportunities

- **Within Phase 2**: T009–T012 and T015 are different files with no shared state
- **Within each story's test block**: every `[P]` test is a separate assertion in the same file and may be written independently, though they land in one file and must be committed together
- **Phase 8**: T070 and T071 are documentation and independent of each other

## Implementation Strategy

**MVP is Phase 1 + 2 + 3.** At that point a developer connects to a bare host and gets a session.
It does nothing useful yet — the engine serves no workspace method until F003 — but the
bootstrap works end to end, which is the thing nothing before this feature could do.

Then US2 and US3 together make the session safe across versions, US4 makes the estate
maintainable, and US5 makes it trustworthy across restarts.

## Notes

- Tasks **T076–T084** were added by the analysis pass and are placed in the phase they belong to,
  so ids are unique but not in numeric order within a phase. Renumbering would have broken the
  cross-references below
- `[P]` marks different files with no incomplete dependency
- Verify each test fails before implementing against it
- Commit at each checkpoint
- Three tests here assert the **absence** of something — no frame on the wire (T032), no override
  path (T042), no execution of an unverified artifact (T017). Absence is the hardest thing to
  test and the easiest to fake, so each states what it asserts on rather than what it hopes for
