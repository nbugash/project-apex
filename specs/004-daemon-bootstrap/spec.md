# Feature Specification: Daemon Bootstrap

**Feature Branch**: `feature/F002-daemon-bootstrap`

**Created**: 2026-09-23

**Status**: Draft

**Input**: Feature map entry F002 `daemon-bootstrap` — engine binary deployment and integrity
verification on first connect, `auth/handshake` with capability exchange, protocol version
negotiation and mismatch policy, in-place binary replacement and re-execution, and recovery of
session state after restart.

## On the source of values

Every normative value in this specification comes from the system specification or from a
recorded decision. Nothing here is invented, and where this document states a number it is
quoting one.

- **§3.8** — deployment over the existing connection, hash verification before execution, and
  the mismatch policy. Decided 2026-09-23 as **A-BOOT**; until that date this was
  `[OPEN: H-BOOT]` and this feature could not be specified at all.
- **§4.8** — `auth/handshake`, its parameters and its result, and `protocolVersion`.
- **§15.3** — in-place binary replacement and re-execution, which is what makes version skew a
  routine condition rather than an exception.
- **A-D12** — toolchain artifacts are pinned, hash-verified and fail closed. Deployment uses the
  same integrity mechanism deliberately, so the project has one way of saying "this binary is
  the one we meant" rather than two.
- **A-EC2** — the remote host is single-tenant and the developer's own, which is why nothing
  here needs elevated privileges.
- **F001** — the transport this feature stands on. It already classifies a missing engine from
  `ssh` exiting 127 and returns `HandToBootstrap`; **nothing consumes that handoff today**, and
  this feature is what does.

## What this feature is not

**It is not the workspace cache.** Nothing here reads or writes repository content. F003 owns
that, and this feature only establishes the session it will run over.

**It does not recover tasks or language servers, because neither exists yet.** The feature map
lists "recovery of active task and LSP session state after restart" under this feature, but
language server multiplexing is F007 and execution terminals are F010 — both far downstream.
Specifying their recovery here would be designing against subsystems nobody can test.

What this feature owns instead is the **session continuity contract** that recovery depends on:
a session identity that survives re-execution, a notification that a restart happened, and a
standing rule that the engine reports what it could not preserve rather than letting it vanish.
F007 and F010 implement their side against that contract when they arrive. This is a narrowing
of the map entry, recorded here rather than silently absorbed.

**It does not carry requests across a lost connection.** A-REQ is unchanged: an in-flight
request dies with its connection. Session continuity across an engine *restart* is a different
question — the connection may survive a re-execution — and nothing in this feature weakens
A-REQ.

**It does not update the client.** A-UPDATE gives that to platform package managers. This
feature only detects the skew and says so.

## Clarifications

### Session 2026-09-23

- Q: What does "recovery of active task and LSP session state" mean when neither tasks nor LSP
  exist at this point in the build order? → A: This feature specifies the session continuity
  contract only — identity that survives re-execution, a restart notification, and a rule that
  unpreserved state is reported. F007 and F010 implement recovery against it.
- Q: The client "carries the engine binary", but the client and the remote host are different
  platforms. Which binary does it carry? → A: The client bundles engine builds for the
  supported remote architectures, detects the host's architecture during connect, and deploys
  the matching one. An unsupported architecture is refused with a clear reason rather than
  deploying something that cannot run.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Connect to a machine that has never run the engine (Priority: P1)

A developer points the application at a host where nothing of this product has ever run. They
do not install anything, copy anything, or open a terminal. The application notices there is no
engine, puts one there, verifies it is the one it meant to send, starts it, and reaches a
working session.

**Why this priority**: This is the feature. F001 can reach a machine and classify the absence of
an engine, and then has nowhere to go — `HandToBootstrap` is a response with no recipient.
Until this story works, the product cannot do anything on a real host, and every feature after
it is blocked.

**Independent Test**: Against a host with no engine present, connect and confirm a session is
established without any manual step, and that the deployed artifact matches what the client
shipped with.

**Acceptance Scenarios**:

1. **Given** a reachable host with no engine installed, **When** the developer connects, **Then**
   the engine is deployed and a session is established with no developer action beyond
   connecting.
2. **Given** a deployment in progress, **When** the developer looks at the application, **Then**
   they can see that it is installing rather than that it has stalled.
3. **Given** an engine that arrives corrupted or truncated, **When** verification runs, **Then**
   it is never executed and the developer is told the deployment failed verification.
4. **Given** a host whose architecture the client has no engine build for, **When** the developer
   connects, **Then** they are told which architecture is unsupported rather than watching a
   binary fail to execute.
5. **Given** an engine already present at the expected version, **When** the developer connects
   again, **Then** nothing is re-transferred and the session starts immediately.

---

### User Story 2 - Know what the engine can do before asking it (Priority: P1)

The client and engine exchange versions and capabilities as the first thing they do. The client
then offers only what this engine actually supports, rather than discovering a gap when a
feature fails in the developer's hands.

**Why this priority**: Capability exchange is what keeps a mixed-version estate honest. Without
it, every unsupported method is discovered as a failed request at the moment a developer tried
to use something, which reads as a broken product rather than an older engine.

**Independent Test**: Handshake against engines advertising different capability sets and
confirm the client's offered functionality changes accordingly, and that a request for an
unadvertised capability never reaches the wire.

**Acceptance Scenarios**:

1. **Given** a connected engine, **When** the session begins, **Then** the handshake completes
   before any other request is sent.
2. **Given** an engine that does not advertise a capability, **When** the client would use it,
   **Then** the request is refused locally and the feature is presented as unavailable rather
   than failing.
3. **Given** a handshake that never receives an answer, **When** its limit elapses, **Then** the
   session fails with a reason distinguishable from a connection failure.

---

### User Story 3 - Refuse a protocol the client does not understand (Priority: P1)

When the engine on the other end speaks a newer protocol than the client knows, the client stops
and says so, rather than proceeding on a guess.

**Why this priority**: This is the safety property that makes the whole deployment scheme sound.
Skew is routine because §15.3 replaces binaries in place, so the mismatch path is not an edge
case — it is a Tuesday. A client that attempts an unknown protocol produces confident wrong
behaviour, which is worse than a clear refusal.

**Independent Test**: Drive the handshake with engines reporting older, identical and newer
protocol versions, and confirm each produces the right outcome: redeploy, proceed, refuse.

**Acceptance Scenarios**:

1. **Given** an engine reporting an older protocol version, **When** the handshake completes,
   **Then** the client redeploys and re-executes it without involving the developer.
2. **Given** an engine reporting a newer protocol version, **When** the handshake completes,
   **Then** the client refuses the session and tells the developer to update the client.
3. **Given** a refusal for a newer engine, **When** the developer looks for a way past it,
   **Then** none is offered — there is no override that proceeds anyway.
4. **Given** an engine reporting the same protocol version, **When** the handshake completes,
   **Then** the session proceeds with no deployment.

---

### User Story 4 - Update the engine without the developer noticing (Priority: P2)

A newer client replaces the engine on a host it has used before, restarts it, and carries on.
The developer is not asked to reconnect and does not lose their place.

**Why this priority**: P2 rather than P1 because the first connect (Story 1) delivers value on
its own — an estate where every engine happens to be current still works. This story is what
keeps it working as the client moves ahead, which matters from the second release onward.

**Independent Test**: Connect with a client newer than the deployed engine and confirm the
replacement, restart and resumed session happen without a developer action, and that a failed
replacement leaves a working engine behind.

**Acceptance Scenarios**:

1. **Given** an engine older than the client, **When** the session is established, **Then** the
   engine is replaced and re-executed without the developer reconnecting.
2. **Given** a replacement that fails to start, **When** the failure is detected, **Then** the
   previous engine is still usable and the developer is told the update failed.
3. **Given** a replacement in progress, **When** another connection attempt arrives, **Then**
   neither attempt leaves a half-written binary that could be executed.

---

### User Story 5 - Keep the session across an engine restart (Priority: P2)

When the engine restarts — because it was updated, or because it crashed and came back — the
client knows it happened, knows what survived, and knows what did not.

**Why this priority**: P2 because the restart itself is delivered by Story 4; this is the
contract that makes it safe for the features that will later have something to lose. Building it
now, while nothing depends on it, is what stops F007 and F010 each inventing their own answer.

**Independent Test**: Force a restart with state registered against the session and confirm the
client is notified, the session identity is unchanged, and anything not preserved is reported
rather than silently dropped.

**Acceptance Scenarios**:

1. **Given** an established session, **When** the engine re-executes itself, **Then** the client
   is notified that a restart occurred rather than inferring it.
2. **Given** a restart, **When** the client resumes, **Then** the session identity is unchanged,
   so anything keyed to the session remains addressable.
3. **Given** state the engine could not carry across the restart, **When** the client resumes,
   **Then** that state is reported as lost rather than appearing to still exist.

---

### Edge Cases

- **The host runs out of disk mid-deployment.** The partial artifact must never be executed, and
  the developer must learn the host is full rather than that the engine is corrupt.
- **Two connections deploy at once.** The same developer with two windows, or a reconnect racing
  a deployment. No sequence of interleavings may leave an executable that is a mixture of two
  transfers.
- **The engine is present but not executable.** Wrong permissions from an interrupted earlier
  run. This must be repaired rather than reported as absence.
- **The engine starts and then wedges without answering the handshake.** Distinct from a
  connection failure and from an engine that never started; the developer needs to know which.
- **A replacement binary is valid but cannot run on this host.** Correct hash, wrong
  architecture or missing system library. The previous engine must remain usable.
- **The engine crashes immediately and repeatedly after a successful deployment.** Redeploying
  in a loop must not be the response to a binary that runs and dies.
- **The client is older than every engine it meets.** The refusal in Story 3 is correct but must
  not be a dead end — the developer is told what to update and how.
- **A deployment succeeds but verification of the landed artifact fails.** Treated as a failed
  deployment, not as a compromised host; the remedy is to retry, and repeated failure is
  reported.

## Requirements *(mandatory)*

### Functional Requirements

**Deployment**

- **FR-001**: The application MUST deploy the engine over the connection it already holds, with
  no additional credential, registry, or outbound network access from the remote host.
- **FR-002**: The application MUST deploy the engine when it is absent, detected from the
  condition F001 already classifies, without requiring the developer to act.
- **FR-003**: The application MUST verify the deployed artifact against the artifact it shipped
  with, before that artifact is executed.
- **FR-004**: A verification failure MUST abort the deployment without executing anything, and
  MUST be reported as a failed deployment.
- **FR-005**: The application MUST place the engine in a location writable by the developer's own
  account, and MUST NOT require elevated privileges at any point.
- **FR-006**: Deployment MUST be atomic with respect to execution: no partially transferred
  artifact may ever be executable, under any interleaving of concurrent attempts.
- **FR-007**: The application MUST NOT re-transfer an engine that is already present and already
  matches the artifact it would send.
- **FR-008**: The application MUST select an engine build matching the remote host's
  architecture, and MUST refuse with a clear reason when it carries no build for that
  architecture.
- **FR-009**: The application MUST make deployment progress visible while it is running, because
  it is a transfer that takes long enough to be mistaken for a hang.

**Handshake and capabilities**

- **FR-010**: The application MUST complete the handshake before issuing any other request on a
  session.
- **FR-011**: The handshake MUST carry the client's version and capabilities, and MUST receive
  the engine's version, protocol version and capabilities.
- **FR-012**: The application MUST record the engine's advertised capabilities for the life of
  the session.
- **FR-013**: The application MUST NOT send a request for a capability the engine did not
  advertise; such a request MUST fail locally and the corresponding functionality MUST be
  presented as unavailable.
- **FR-014**: A handshake that does not complete within its limit MUST fail the session with a
  condition distinguishable from a transport failure.

**Version negotiation**

- **FR-015**: The application MUST treat the client as the authority on protocol version.
- **FR-016**: An engine reporting an older protocol version MUST be replaced and re-executed
  automatically.
- **FR-017**: An engine reporting a newer protocol version MUST cause the session to be refused,
  with an instruction to update the client.
- **FR-018**: The refusal in FR-017 MUST NOT be overridable. No option may proceed with an
  unknown protocol.
- **FR-019**: An engine reporting the same protocol version MUST proceed without deployment.

**Replacement and restart**

- **FR-020**: The engine MUST support being replaced and re-executed in place, without the
  developer reconnecting.
- **FR-021**: A replacement that fails to start MUST leave a working engine in place, and MUST
  be reported as a failed update rather than as an absent engine.
- **FR-022**: The application MUST stop redeploying after three consecutive attempts in which
  the engine starts and exits before completing a handshake, and MUST report that the engine
  cannot run on this host. "Indefinitely" is not a testable bound, and a redeploy loop against a
  binary that runs and dies is indistinguishable from a hang.
- **FR-023**: The engine MUST notify the client that a restart occurred, rather than leaving the
  client to infer it.
- **FR-024**: A session's identity MUST survive re-execution, so that anything keyed to the
  session remains addressable across it.
- **FR-025**: The engine MUST report state it could not preserve across a restart. State that is
  lost MUST NOT appear to have survived.

**Verification**

- **FR-026**: Every behaviour in this specification MUST be verifiable without a remote host, a
  network, or a real engine, consistent with the standard F001 established and A-TEST made
  binding.

### Key Entities

- **Engine artifact**: The binary the client carries and deploys. Identified by its version, its
  target architecture, and a digest that is the sole means of deciding whether what landed is
  what was sent.
- **Deployment**: One attempt to place an artifact on a host. Has a staged location and a final
  location, and becomes executable only on passing verification.
- **Handshake**: The first exchange on a session. Carries versions and capabilities in both
  directions and produces either an established session or a named refusal.
- **Protocol version**: An integer that increments on any breaking change to the method
  catalogue. Compared, never negotiated — the client is the authority.
- **Capability set**: What each side declares it can do. The client's offered functionality is a
  function of the engine's set.
- **Session**: The established relationship between client and engine. Carries an identity that
  survives re-execution of the engine.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer connects to a host that has never run the engine and reaches a usable
  session with zero manual steps — no terminal, no copied file, no installed package.
- **SC-002**: First connect to a host with no engine present completes within 30 seconds on a
  10 Mbit/s link, excluding any time spent waking the instance.
- **SC-003**: A second connect to a host already running the current engine establishes a session
  without transferring the artifact again.
- **SC-004**: An artifact that fails verification is executed zero times. Not "rarely" — the
  count is zero across every failure mode exercised.
- **SC-005**: A client never exchanges a request with an engine whose protocol version exceeds
  its own.
- **SC-006**: An engine update completes without the developer reconnecting, and without losing
  the session they were working in.
- **SC-007**: A failed update leaves a working engine in 100% of exercised failure modes.
- **SC-008**: A capability the engine does not advertise produces no request on the wire, in 100%
  of cases.
- **SC-009**: Every restart is reported to the client; the client never learns of one by
  inference.
- **SC-010**: The full suite for this feature runs with no remote host, no network and no real
  engine.
- **SC-011**: An engine that starts and dies is redeployed at most three times before the
  developer is told it cannot run here, rather than the application retrying while appearing to
  hang.
- **SC-012**: Concurrent deployment attempts produce either one valid engine or a reported
  failure, never a mixed artifact, across a sustained run of interleaved attempts.

## Assumptions

- **The developer can write to their own home directory on the remote host.** Guaranteed by
  A-EC2's single-tenant, per-developer instance. A shared or locked-down host would reopen
  FR-005.
- **The client ships engine builds for the supported remote architectures.** Linux x86-64 and
  ARM64 are assumed sufficient, since A-EC2 makes the remote an EC2 instance the project
  provisions. This has a consequence for F014: the client package carries the engine, more than
  once.
- **The engine binary is small enough to transfer over the control channel without breaching the
  interaction budget.** A-BOOT records the opposite as its reversal condition, so if this proves
  false the deployment mechanism is what changes, not this feature's requirements.
- **The remote host has no outbound internet access.** Assumed rather than required — the design
  deliberately does not depend on it, which is the reason A-BOOT rejected a download-from-URL
  approach.
- **Tasks and language servers do not exist yet.** F007 and F010 deliver them. This feature
  builds the contract their recovery will use and tests it against the mock.
- **F001's transport is available and unchanged.** This feature consumes its connection, its
  failure classification and its request correlation, and adds no requirement to them.
