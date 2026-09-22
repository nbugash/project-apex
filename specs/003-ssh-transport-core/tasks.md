# Tasks: SSH Transport Core

**Branch**: `feature/F001-ssh-transport-core` | **Date**: 2026-09-22 | **Plan**: [plan.md](./plan.md)

**Input**: Design documents from `/specs/003-ssh-transport-core/`

## Format: `[ID] [P?] [Story] Description`

- **[P]**: different files, no dependency on an unfinished task — may run in parallel
- **[Story]**: the user story this task serves, for story phases only

## Path Conventions

Paths are the ones in [design.md](./design.md)'s Module & File Layout, which matches
plan.md's Structure Decision. Rust core only: this feature adds no interface-layer surface.

## Tests are required, not optional

Constitution Principle VII requires tests at every level where the feature has surface, and
names **protocol framing** among the places where writing them first is expected because a
wrong answer is expensive. The framing and correlation tests below are therefore written
**before** their implementations and must fail first. Tests that have never failed
demonstrate nothing about whether they would catch the bug.

---

## Phase 1: Setup (Shared Infrastructure)

- [ ] T001 Add the `process` and `io-util` tokio features and the `bytes` dependency to `src-tauri/Cargo.toml`, keeping the existing feature set intact
- [ ] T002 [P] Declare the `apex-askpass` binary target in `src-tauri/Cargo.toml` pointing at `src-tauri/bin/apex-askpass.rs`
- [ ] T003 [P] Declare the mock daemon as a test-only binary target in `src-tauri/Cargo.toml` pointing at `src-tauri/tests/mock_daemon/main.rs`
- [ ] T004 Create the empty module tree — `src-tauri/src/adapters/outbound/openssh/{mod,spawner,framing,registry,sendq,classify}.rs` and `src-tauri/src/adapters/outbound/askpass/ipc.rs` — wired into their parent `mod.rs` files so the crate still builds

---

## Phase 2: Foundational (Blocking Prerequisites)

**Blocks every user story.** The mock daemon lives here rather than in User Story 5 because
every other story's tests are driven through it; US5 asserts the _properties_ the mock makes
possible, not the mock's existence.

### Domain types

- [ ] T005 [P] Define `RequestId`, `Priority` and `RequestOutcome` in `src-tauri/src/domain/request.rs` per [data-model.md](./data-model.md)
- [ ] T006 [P] Define `FailureCondition` with all seven variants in `src-tauri/src/domain/failure.rs` per [data-model.md](./data-model.md)
- [ ] T007 Extend `ConnectionState` with `Retrying { attempt, next_at }` in `src-tauri/src/domain/connection.rs`, preserving the four existing variants F000's status bar already renders
- [ ] T008 [P] Unit tests for the `ConnectionState` transition table in `src-tauri/src/domain/connection.rs`, including that `HostKeyChanged` reaches `Disconnected` and never `Retrying`
- [ ] T009 [P] Define `Secret` in `src-tauri/src/domain/request.rs` — zeroes its buffer on drop, redacts on `Debug` and `Display` (FR-008)
- [ ] T010 [P] Unit test asserting `Secret` renders no plaintext through `Debug`, `Display` or a formatted panic payload, in `src-tauri/src/domain/request.rs`

### Ports

- [ ] T011 [P] Define the `RequestTransport` trait in `src-tauri/src/application/ports/transport.rs` per [contracts/transport.md](./contracts/transport.md) and [design.md](./design.md)
- [ ] T012 [P] Define the `ProcessSpawner` trait and `SpawnSpec`/`SpawnedChild` in `src-tauri/src/application/ports/spawner.rs`
- [ ] T013 [P] Define the `CredentialPrompt` trait and `PromptContext`/`PromptError` in `src-tauri/src/application/ports/credential.rs`

### Framing codec — tests first (Principle VII)

- [ ] T014 Write the failing conformance tests for `FrameCodec::decode` in `src-tauri/src/adapters/outbound/openssh/framing.rs`, one per row of the table in [contracts/framing.md](./contracts/framing.md). **These must fail before T016 exists.** The three rows that are not hostile input — header split across reads, body split across reads, two frames in one read — matter most: an implementation that assumes one read yields one frame fails intermittently under load rather than reliably in a test
- [ ] T015 Write the failing test asserting a declared length above the cap is refused **before** any allocation, in `src-tauri/src/adapters/outbound/openssh/framing.rs`. Checking the cap after allocating defeats the defence entirely
- [ ] T016 Implement `FrameCodec` encode and decode in `src-tauri/src/adapters/outbound/openssh/framing.rs` until T014 and T015 pass, reading the format from §4.1 and restating none of its constants

### Mock daemon

- [ ] T017 Implement the mock daemon's framing loop in `src-tauri/tests/mock_daemon/main.rs` — read a frame, echo a reply — reusing `FrameCodec` so mock and transport cannot disagree about the wire
- [ ] T018 Add scripted behaviours to `src-tauri/tests/mock_daemon/main.rs`: delay a reply, drop a reply, emit a malformed frame, emit an oversized length, close the pipe mid-frame
- [ ] T019 Add per-frame delay and loss to `src-tauri/tests/mock_daemon/main.rs`, configurable, defaulting to the 250 ms / 5% profile the feature map names
- [ ] T020 [P] Implement `ScriptedSpawner` in `src-tauri/tests/mock_daemon/spawner.rs` — produces a chosen exit code and chosen stderr without running `ssh`

**Checkpoint**: the codec is proven against hostile and awkward input, and the mock can be
driven into every condition the later stories need.

---

## Phase 3: User Story 1 - Reach the remote machine (Priority: P1) 🎯 MVP

**Goal**: one authenticated connection per session, reused by every channel, torn down on
quit, and re-established when it drops.

**Independent test**: point the transport at the mock, connect, confirm one connection serves
a second channel, quit, confirm nothing is left running; then kill the mock and watch the
transport recover.

### Tests for User Story 1

- [ ] T021 [P] [US1] Integration test in `src-tauri/tests/transport_recovery.rs`: one connection established, reused by a second logical channel with no second authentication (SC-002)
- [ ] T022 [P] [US1] Integration test in `src-tauri/tests/transport_recovery.rs`: after teardown, no child process survives (SC-003)
- [ ] T023 [P] [US1] Integration test in `src-tauri/tests/transport_recovery.rs`: the mock closes the pipe, every outstanding request resolves as `ConnectionLost`, and none hangs (SC-012)
- [ ] T024 [P] [US1] Integration test in `src-tauri/tests/transport_recovery.rs`: the retry interval **grows** between attempts. A tight retry loop passes a "did it reconnect" assertion and is still wrong
- [ ] T025 [P] [US1] Integration test in `src-tauri/tests/transport_recovery.rs`: the connection re-establishes with no caller action once the mock accepts again (SC-012)

### Implementation for User Story 1

- [ ] T026 [US1] Implement `OpenSshSpawner` in `src-tauri/src/adapters/outbound/openssh/spawner.rs` — the §3.1 invocation, flag for flag, in this one place and nowhere else
- [ ] T027 [US1] Implement startup preflight in `src-tauri/src/adapters/outbound/openssh/spawner.rs`: verify `ssh` is present and report its version, refusing below 6.7 with a message naming what was found (FR-005)
- [ ] T028 [US1] Implement the reader and writer tasks in `src-tauri/src/adapters/outbound/openssh/mod.rs`, as separate tasks so a blocked writer cannot stop replies being read
- [ ] T029 [US1] Implement `Supervisor` in `src-tauri/src/application/use_cases/supervise.rs`: own the child's lifetime, detect loss by **both** EOF and keepalive expiry, publish `ConnectionState`
- [ ] T030 [US1] Implement the backoff schedule in `src-tauri/src/application/use_cases/supervise.rs` — 1 s doubling to a 30 s ceiling with full jitter, retrying indefinitely, per [research.md](./research.md)
- [ ] T031 [US1] Implement teardown in `src-tauri/src/adapters/outbound/openssh/mod.rs`: issue `ssh -O exit` on quit so no master outlives the application (§3.1)

**Checkpoint**: the connection exists, survives a drop, and leaves nothing behind.

---

## Phase 4: User Story 2 - Authenticate without leaving the application (Priority: P1)

**Goal**: agent-held keys connect silently; passphrase-protected keys prompt inside the app;
neither leaves the user in a terminal.

**Independent test**: connect three ways against the mock — agent, passphrase, neither — and
confirm each reaches the right outcome with no terminal.

### Tests for User Story 2

- [ ] T032 [P] [US2] Integration test in `src-tauri/tests/transport_failures.rs`: a credential the agent holds connects with no prompt at all
- [ ] T033 [P] [US2] Integration test in `src-tauri/tests/transport_failures.rs`: the assisted phase is entered **only** from `AuthenticationFailed` — an unreachable host and a missing engine must never raise a passphrase prompt (FR-006)
- [ ] T034 [P] [US2] Integration test in `src-tauri/tests/transport_failures.rs`: after an assisted connect, a sentinel passphrase appears in no log, no error and no panic payload (FR-008, SC-001)
- [ ] T035 [P] [US2] Integration test in `src-tauri/tests/transport_failures.rs`: a cancelled prompt ends the attempt cleanly rather than waiting forever

### Implementation for User Story 2

- [ ] T036 [US2] Implement the local IPC channel in `src-tauri/src/adapters/outbound/askpass/ipc.rs` that the helper reaches back through
- [ ] T037 [US2] Implement the helper in `src-tauri/bin/apex-askpass.rs` — read the prompt, ask the running app, write the answer to stdout, zero the buffer. Deliberately tiny: all judgement stays in the app
- [ ] T038 [US2] Implement the two-phase connect sequence in `src-tauri/src/application/use_cases/connect.rs`: silent with `BatchMode`, then assisted with `SSH_ASKPASS` as an **absolute** path and `SSH_ASKPASS_REQUIRE=force` (§3.3)
- [ ] T039 [US2] Implement the identity-picker fallback in `src-tauri/src/application/use_cases/connect.rs` for when every automatic route fails (FR-009)
- [ ] T040 [US2] Degrade gracefully below OpenSSH 8.4 in `src-tauri/src/application/use_cases/connect.rs`: attempt the silent route, then offer the picker, rather than appearing to hang (§3.3)

**Checkpoint**: every credential situation reaches an outcome without a terminal.

---

## Phase 5: User Story 3 - Exchange requests without losing or crossing them (Priority: P1)

**Goal**: many concurrent requests, each answer reaching its own request; unanswered requests
give up; superseded requests are withdrawn; nothing accumulates.

**Independent test**: issue many concurrent requests against the mock with replies
deliberately reordered, plus some never answered and some withdrawn, and confirm every
outcome lands correctly and the registry empties.

### Tests for User Story 3

- [ ] T041 [P] [US3] Failing test in `src-tauri/tests/transport_exchange.rs`: with many requests in flight and replies reordered, every outcome reaches its own request and none is misdelivered (SC-004). **Write before T046**
- [ ] T042 [P] [US3] Failing test in `src-tauri/tests/transport_exchange.rs`: a reply arriving the instant a write completes is still matched, proving registration precedes transmission (FR-011)
- [ ] T043 [P] [US3] Test in `src-tauri/tests/transport_exchange.rs`: after a sustained run of answered, timed-out and withdrawn requests, the registry retains nothing. Assert on the registry's **size**, not on the run completing — a leak does not fail a short test (SC-005)
- [ ] T044 [P] [US3] Test in `src-tauri/tests/transport_exchange.rs`: a request the mock never answers resolves as `TimedOut` within its limit and stops occupying the connection (SC-006)
- [ ] T045 [P] [US3] Test in `src-tauri/tests/transport_exchange.rs`: a withdrawn request resolves as `Withdrawn`, a cancellation is sent, and withdrawing an unknown or already-resolved id is a no-op
- [ ] T046 [P] [US3] Test in `src-tauri/tests/transport_exchange.rs`: with background requests saturating the queue, an interactive request is written ahead of all queued background work — but not ahead of a frame already being written (SC-013)
- [ ] T047 [P] [US3] Test in `src-tauri/tests/transport_exchange.rs`: a payload over the frame cap is refused before transmission, and a normal request **afterwards** still succeeds, proving the stream stayed aligned

### Implementation for User Story 3

- [ ] T048 [US3] Implement `Registry` in `src-tauri/src/adapters/outbound/openssh/registry.rs`: mint ids, register before write, resolve exactly once, remove on every outcome
- [ ] T049 [US3] Implement deadline expiry in `src-tauri/src/adapters/outbound/openssh/registry.rs` with the 30-second default and a per-call override (FR-012)
- [ ] T050 [US3] Implement `Registry::fail_all` in `src-tauri/src/adapters/outbound/openssh/registry.rs`, called by the supervisor on connection loss so nothing waits for a dead link (FR-020)
- [ ] T051 [US3] Implement `SendQueue` in `src-tauri/src/adapters/outbound/openssh/sendq.rs`: `Interactive` ahead of `Background`, FIFO within a class (FR-021)
- [ ] T052 [US3] Implement the exchange use case in `src-tauri/src/application/use_cases/exchange.rs` — send, correlate, expire, withdraw — wiring registry and queue behind `RequestTransport`
- [ ] T053 [US3] Implement cancellation emission in `src-tauri/src/application/use_cases/exchange.rs` per §4.5, and resolve the withdrawn request regardless of whether the remote side stops

**Checkpoint**: the transport carries concurrent traffic correctly under reordering, loss and
withdrawal.

---

## Phase 6: User Story 4 - Understand why a connection failed (Priority: P2)

**Goal**: each failure names itself, and the two with consequences — a changed host key, a
missing engine — are never blurred into a generic failure.

**Independent test**: drive `ScriptedSpawner` into each condition and confirm the
classification and the response that belongs to it.

### Tests for User Story 4

- [ ] T054 [P] [US4] Test in `src-tauri/tests/transport_failures.rs`: each of the seven `FailureCondition` variants classifies as itself, none as a generic failure (SC-007)
- [ ] T055 [P] [US4] Test in `src-tauri/tests/transport_failures.rs`: the same stderr under a non-English locale classifies identically (SC-008)
- [ ] T056 [P] [US4] Test in `src-tauri/tests/transport_failures.rs`: `HostKeyChanged` refuses the connection and **never retries**. If it retries, that is a security defect, not a flaky test
- [ ] T057 [P] [US4] Test in `src-tauri/tests/transport_failures.rs`: a missing engine classifies as `EngineMissing` and is not reported as a connection failure (FR-017)
- [ ] T058 [P] [US4] Test in `src-tauri/tests/transport_failures.rs`: stderr beyond the retained bound degrades to `Unknown` rather than exhausting memory or misclassifying

### Implementation for User Story 4

- [ ] T059 [US4] Implement `classify` in `src-tauri/src/adapters/outbound/openssh/classify.rs`: exit code plus a bounded 8 KiB of `LC_ALL=C` stderr matched against a fixed pattern set, yielding `Unknown` when nothing matches
- [ ] T060 [US4] Wire each condition to its response in `src-tauri/src/application/use_cases/supervise.rs` — retry, refuse, or hand off — per the table in [data-model.md](./data-model.md)
- [ ] T061 [US4] Implement the explicit changed-host-key warning and the user-confirmed forget action in `src-tauri/src/application/use_cases/connect.rs`, never automatic (§3.9, FR-016)

**Checkpoint**: every failure is actionable and the dangerous one is unmistakable.

---

## Phase 7: User Story 5 - Verify the transport without a remote machine (Priority: P2)

**Goal**: the whole suite runs with no network, no remote host and no engine, including
under latency and loss.

**Independent test**: disable networking and run the suite.

### Tests for User Story 5

- [ ] T062 [P] [US5] Test in `src-tauri/tests/transport_exchange.rs`: over the mock's 250 ms / 5% profile, no replies are lost or crossed
- [ ] T063 [US5] Measure the transport's **added** overhead in `src-tauri/tests/transport_exchange.rs` — request handed in to answer handed back, excluding the simulated round trip — and fail above 15 ms at the 99th percentile (SC-011). Measuring wall clock would pass regardless of what the transport does, because the harness's own 250 ms dominates it
- [ ] T064 [US5] Add a suite-level assertion in `src-tauri/tests/transport_exchange.rs` that no test opened a network socket or required a remote host (SC-010)

### Implementation for User Story 5

- [ ] T065 [US5] Document the mock's scripted behaviours and configuration in `src-tauri/tests/mock_daemon/README.md`, so a later feature can drive it without reading its source
- [ ] T066 [US5] Assert in `src-tauri/tests/mock_daemon/main.rs` that the mock implements no §4.8 method, keeping it a framing double rather than a second engine

**Checkpoint**: the feature is provable on a laptop with the network off.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [ ] T067 Replace `StubConnectionStatusSource` with the real transport in `src-tauri/src/composition.rs` — the one-line swap the port structure exists to buy. Keep the stub for tests
- [ ] T068 [P] Verify the interface layer renders `Retrying` sensibly in `src/lib/statusbar/StatusBar.svelte`, showing that a reconnect is in progress rather than a state it does not recognise
- [ ] T069 [P] Document the transport — ports, the mock, how to drive it, and the failure table — in `docs/transport.md`
- [ ] T070 [P] Add an **opt-in** integration test against a locally spawned `sshd` in `src-tauri/tests/transport_real_sshd.rs`, skipped with a clear reason when `sshd` is unavailable. Principle VII names "the protocol against a real `sshd`" as the integration standard, and SC-010 forbids the suite from _requiring_ one; opt-in satisfies both rather than quietly choosing one over the other
- [ ] T071 Run the full quickstart validation and record the results in [quickstart.md](./quickstart.md)
- [ ] T072 Run `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` and `cargo fmt --manifest-path src-tauri/Cargo.toml --check`, and fix what they report across `src-tauri/`
- [ ] T073 Promote the two decisions marked in [research.md](./research.md) to Appendix A of `project-apex-predator.md` — that an in-flight request dies with the connection, and that outbound priority is stated by the caller. Both bind every later feature, and a decision only F001's research records is one F002 will not find

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)** → no dependencies
- **Foundational (Phase 2)** → Setup. **Blocks every story**: the codec and the mock are what every later test runs through
- **US1 (Phase 3)** → Foundational
- **US2 (Phase 4)** → Foundational + US1 (the connect sequence needs a spawner and a supervisor to sequence)
- **US3 (Phase 5)** → Foundational + US1 (a connection must exist before requests cross it)
- **US4 (Phase 6)** → Foundational; independent of US2 and US3 because `ScriptedSpawner` replaces the real connection entirely
- **US5 (Phase 7)** → US3 (it measures the exchange path)
- **Polish (Phase 8)** → all stories

### User Story Dependencies

US1 is the spine. US2 and US3 both build on it and are independent of **each other** — one is
about establishing a connection, the other about using one. US4 is the most independent of
all and could be built first if failure classification were the riskiest unknown.

### Parallel Opportunities

- **Phase 2**: T005, T006, T008, T009, T010 in parallel (separate files); T011–T013 in
  parallel; T020 alongside T017–T019
- **Phase 3**: T021–T025 in parallel — all new tests in one file, but independent cases
- **Phase 4**: T032–T035 in parallel
- **Phase 5**: T041–T047 in parallel
- **Phase 6**: T054–T058 in parallel; the whole phase parallel with US2 and US3
- **Phase 8**: T068, T069, T070 in parallel

---

## Parallel Example: User Story 3

```text
Together (independent test cases):
  T041  correlation under reordering
  T042  registration precedes transmission
  T043  registry retains nothing
  T044  timeout
  T045  withdrawal
  T046  priority ordering
  T047  oversized payload, stream stays aligned

Then sequentially (shared files, real dependencies):
  T048 → T049 → T050   registry.rs
  T051                 sendq.rs
  T052 → T053          exchange.rs
```

---

## Implementation Strategy

### MVP First (User Story 1 only)

Phases 1–3 give a connection that establishes, is reused, tears down cleanly and recovers
from a drop. That is demonstrable on its own and is what every other story builds on.

It is **not** shippable as a product increment — it carries no requests. The MVP boundary
here is about proving the riskiest thing first, not about user-visible value, because this
whole feature is infrastructure.

### Incremental Delivery

1. **Phases 1–2** — the codec is proven against hostile input and the mock exists
2. **Phase 3 (US1)** — a connection with a lifetime
3. **Phase 5 (US3)** — requests cross it correctly
4. **Phase 4 (US2)** — real credentials, not just the mock's
5. **Phase 6 (US4)** — failures become actionable
6. **Phase 7 (US5)** — the guarantees are measured, not asserted
7. **Phase 8** — the stub is retired and the decisions are promoted

US3 before US2 is deliberate: the exchange path is the riskiest part of the feature and the
one F002 depends on most. Authentication is well-trodden ground that OpenSSH does for us.

### Parallel Team Strategy

US4 is the clean split: `ScriptedSpawner` means it needs no real connection, so failure
classification can be built alongside US1 and US3 without either waiting.

---

## Notes

- **Fail first where it is cheap to be wrong**: T014, T015, T041 and T042 must fail before
  their implementations exist. Principle VII names framing explicitly.
- **The stub stays.** T067 retires `StubConnectionStatusSource` from the composition root,
  not from the codebase — it remains useful for tests that want a connection without a mock.
- **No interface work.** This feature adds no rendered surface; T068 only checks that a state
  F000 already renders gained a variant it can display.
- **`[OPEN: H-BOOT]` is out of scope by design.** T057 asserts a missing engine classifies
  and hands off. Installing one is F002.
