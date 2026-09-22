# Feature Specification: SSH Transport Core

**Feature Branch**: _none — no branch extension installed; spec directory is `specs/003-ssh-transport-core`_

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "F001" — resolved through the feature map sequence gate to
`F001 ssh-transport-core`, whose pending subfeatures define this scope: OpenSSH subprocess
invocation with master connection lifecycle and startup preflight, the two-phase connect
sequence with the bundled askpass helper, a `Content-Length` framing codec over the child
process stdio, a request correlation registry with timeouts and cancellation, failure
classification from exit codes and locale-pinned stderr, and a mock SSH daemon harness
simulating 250 ms round-trip time and 5% packet loss.

## On the source of values

The transport mechanism is already decided. Appendix A, A-B1 of the system specification
records it, and §3.1 carries the normative invocation flag by flag. This specification does
not restate that invocation, the flags or their reasons.

That is deliberate, and it is the same discipline the chrome specification follows: copying a
normative decision into a second document creates a second source of truth that drifts on the
first change. Where this document needs to refer to the transport's shape, it refers to the
system specification.

What this document adds is the part §3 does not state: what a developer experiences, what
must be true for the feature to be finished, and how that is judged.

## What this feature is not

This is the boundary that keeps the feature buildable, and it is worth stating plainly
because the transport touches everything.

**It does not put the engine on the remote host.** `[OPEN: H-BOOT]` in §3.8 — how the engine
binary arrives, how its version is verified, and what happens when it is absent — belongs to
F002 daemon-bootstrap. This feature detects that the engine is missing and reports it as a
distinct, named condition; it does not install anything.

**It does not speak the application protocol.** No handshake, no capability exchange, no
method from the §4.8 catalogue. This feature carries frames and matches replies to requests.
What is inside a frame is the business of the features that send them.

**It does not forward ports or move bulk data.** Preview forwarding (§3.5) and the SFTP
channel (§3.6) both attach to the master connection this feature establishes, which is why
the master's lifecycle is in scope here, but neither is built here.

The consequence is that this feature is finished and verifiable before any remote engine
exists, against the mock daemon in User Story 5. That is the reason the mock is a deliverable
rather than test scaffolding.

## Clarifications

### Session 2026-09-22

- Q: When the network drops mid-session, does F001 reconnect automatically, or only detect and report the loss? → A: F001 reconnects, with backoff. F012 adds the offline experience on top of a transport that already heals.
- Q: The system spec's interaction budget names no percentile or measurement point ([OPEN: NFR]). What should F001's latency criterion actually measure? → A: The 99th percentile of the transport's _added_ overhead — handing a request in to getting its answer back, excluding the remote side's processing and the link's own round trip.
- Q: System spec §4.6 requires outbound frames to be priority-queued, with editor traffic ahead of background work. Does F001 build that, or does it ship FIFO? → A: F001 builds it. §4.6 is normative and transport-level, and no other feature is assigned it.
- Q: What should a request's default time limit be before it gives up? → A: 30 seconds, overridable per call site.

## User Scenarios & Testing _(mandatory)_

### User Story 1 - Reach the remote machine (Priority: P1)

A developer opens a remote workspace. The application establishes one authenticated
connection to the host and holds it open for the session. Every later channel reuses it;
nothing re-authenticates. When the developer quits, the connection is torn down rather than
left running.

**Why this priority**: Nothing else in the product exists without it. Every remote feature is
a passenger on this connection.

**Independent Test**: Point the application at the mock daemon, connect, confirm a single
authenticated connection is established and reused by a second logical channel, then quit and
confirm nothing is left running.

**Acceptance Scenarios**:

1. **Given** a reachable host and a usable credential, **When** the developer connects,
   **Then** the connection is established and the application reports itself connected.
2. **Given** an established connection, **When** a second logical channel is opened, **Then**
   it attaches to the existing connection without a further authentication.
3. **Given** an established connection, **When** the developer quits the application, **Then**
   the connection is torn down and no background process outlives the application.
4. **Given** the host stops responding mid-session, **When** roughly 45 seconds pass, **Then**
   the application reports the connection lost rather than waiting indefinitely.
5. **Given** a machine whose SSH client is absent or too old to support the connection-reuse
   path, **When** the application starts, **Then** it says so at startup with a clear message,
   rather than failing at the first connection attempt.
6. **Given** the connection is lost mid-session, **When** the host becomes reachable again,
   **Then** the transport re-establishes it without the developer restarting the application.
7. **Given** the host stays unreachable, **When** the transport retries, **Then** the interval
   between attempts grows rather than retrying in a tight loop, and the developer can see
   that it is still trying.

---

### User Story 2 - Authenticate without leaving the application (Priority: P1)

A developer whose key is already held by their agent connects with no prompt at all. A
developer whose key has a passphrase is asked for it inside the application, once. A developer
with neither can choose a key file directly.

**Why this priority**: Equal to Story 1, because a connection nobody can authenticate is not a
connection. This is also the only part of the transport a developer interacts with directly,
and the part where a poor experience is most visible — an IDE that drops the user into a
terminal to authenticate has failed at the thing it exists to do.

**Independent Test**: Connect three ways against the mock daemon — with the credential held by
the agent, with a passphrase-protected key, and with neither — and confirm each reaches the
right outcome without a terminal.

**Acceptance Scenarios**:

1. **Given** the credential is already held by the agent, **When** the developer connects,
   **Then** they are not prompted for anything.
2. **Given** a passphrase-protected credential, **When** the developer connects, **Then** the
   passphrase is requested inside the application and the connection proceeds on a correct
   answer.
3. **Given** authentication fails for every automatic route, **When** the attempts are
   exhausted, **Then** the developer is offered a choice of credential file rather than an
   error they cannot act on.
4. **Given** a passphrase has been supplied, **When** the connection is established, **Then**
   the passphrase does not appear in any log, diagnostic or crash report.
5. **Given** a platform too old to support prompting inside the application, **When** the
   developer connects, **Then** the automatic routes are attempted and the credential picker
   is offered, rather than the connection appearing to hang.

---

### User Story 3 - Exchange requests without losing or crossing them (Priority: P1)

A developer's editor issues many small requests at once — completions, diagnostics, file
reads — and each answer arrives at the request that asked for it. A request that will never be
answered gives up rather than hanging. A request that is superseded is withdrawn rather than
left to consume the link.

**Why this priority**: Equal to Stories 1 and 2. A connection that carries frames but crosses
their replies is worse than no connection, because the failure is silent and intermittent.
This is also the component the system specification names as the first to build (§4.3).

**Independent Test**: Issue many concurrent requests against the mock daemon, including some
the mock never answers and some the client withdraws, and confirm every answer reaches its
own request, every unanswered request gives up, and nothing accumulates.

**Acceptance Scenarios**:

1. **Given** many requests in flight at once, **When** the answers arrive in an order
   unrelated to the order asked, **Then** each answer is delivered to the request that asked
   for it.
2. **Given** a request the remote side never answers, **When** its time limit passes, **Then**
   it resolves as a failure and stops occupying the connection.
3. **Given** a request the developer's action has superseded, **When** it is withdrawn,
   **Then** it stops consuming the link and resolves rather than being silently dropped.
4. **Given** a long session of many thousands of requests, **When** it ends, **Then** nothing
   is retained for requests that are already finished.
5. **Given** a payload larger than the link is meant to carry, **When** it is sent or
   received, **Then** it is refused as a named error rather than stalling everything behind
   it.

---

### User Story 4 - Understand why a connection failed (Priority: P2)

A developer whose connection fails is told which of a small number of things went wrong — the
host is unreachable, the credential was refused, the host's identity changed, the engine is
not installed, the engine crashed, the network dropped — and what follows from it.

**Why this priority**: Below the first three because a connection that works does not need
this. Above nothing, because the alternative is a single "connection failed" that sends every
user to the same dead end, and because two of these conditions have consequences that must not
be blurred: an engine that is missing is a routine first-run state, while a host identity that
has changed may be an attack.

**Independent Test**: Drive the mock daemon into each failure condition in turn and confirm the
application names the right one and takes the response that belongs to it.

**Acceptance Scenarios**:

1. **Given** each distinct failure condition, **When** it occurs, **Then** the application
   reports that condition specifically, not a generic failure.
2. **Given** the host's identity has changed, **When** the connection is refused, **Then** the
   developer is warned explicitly that this may be an attack, and the connection is not
   established.
3. **Given** the host's identity has legitimately changed, **When** the developer chooses to
   forget the old identity, **Then** it is forgotten only on that explicit confirmation, never
   automatically.
4. **Given** the engine is not installed on the host, **When** the connection is attempted,
   **Then** that specific condition is reported and handed to the feature that installs it.
5. **Given** the system is running in a language other than English, **When** a failure occurs,
   **Then** it is classified identically to the same failure in English.

---

### User Story 5 - Verify the transport without a remote machine (Priority: P2)

An engineer changes the transport and finds out whether they broke it, on their own machine,
without an EC2 instance, a network or a remote engine — including under a slow, lossy link.

**Why this priority**: Below the transport itself, because there must be something to test.
Above nothing, because a transport whose failure modes are only reachable on real infrastructure
is a transport whose failure modes are never tested. Latency and loss are where this component
actually breaks, and they are not reproducible on a developer's loopback by accident.

**Independent Test**: Run the transport's tests with no network access and no remote host, and
confirm they exercise a link with meaningful delay and dropped packets.

**Acceptance Scenarios**:

1. **Given** a machine with no network access and no remote host, **When** the transport's
   tests run, **Then** they complete and exercise the full connect, exchange and failure
   paths.
2. **Given** the mock, **When** a test requires it, **Then** it can be made to exhibit each of
   the failure conditions in Story 4 on demand.
3. **Given** a link with significant delay and loss, **When** requests are exchanged over it,
   **Then** the transport's behaviour under those conditions is what is measured.

---

### Edge Cases

- **The connection is already established when the application starts.** A connection left
  behind by a previous run is reused rather than duplicated, or is replaced — never left
  orphaned while a second one is created alongside it.
- **The developer quits while requests are in flight.** In-flight requests resolve as
  cancelled rather than hanging, and teardown still happens.
- **The remote side writes diagnostics onto the same channel as its errors.** Diagnostic text
  from the engine must not be mistaken for the transport's own failure signals; the
  classification in Story 4 must remain correct when the engine is noisy.
- **A reply arrives for a request that has already given up.** It is discarded without
  disturbing anything still in flight.
- **A reply arrives before the client has finished registering the request.** It is still
  matched; a reply must not be able to overtake its own registration.
- **The remote side sends a malformed frame.** It is refused as a protocol error and does not
  desynchronise the stream for everything after it.
- **Two requests are given the same identifier.** Treated as a defect and refused, rather than
  silently delivering one answer to two callers.
- **The host returns while requests are still outstanding from before the drop.** Requests
  that were in flight when the connection died resolve as failed rather than silently waiting
  for a reply that can never arrive on a connection that no longer exists.
- **The connection drops repeatedly.** Retrying does not escalate into a tight loop, and
  repeated failure is distinguishable from a first one.
- **Background traffic is queued when an interactive request arrives.** The interactive
  request goes ahead of it; a large background payload already being written does not have to
  finish first for the interactive one to start.
- **The passphrase prompt is cancelled by the developer.** The connection attempt ends
  cleanly; the application does not wait forever for an answer that will not come.

## Requirements _(mandatory)_

### Functional Requirements

- **FR-001**: The application MUST establish a single authenticated connection to the remote
  host per session, using the invocation the system specification makes normative in §3.1.
- **FR-002**: Every logical channel MUST reuse that connection; nothing may re-authenticate
  mid-session.
- **FR-003**: The application MUST tear the connection down on quit, leaving no process
  outliving it.
- **FR-004**: The application MUST detect a host that has stopped responding within the
  keepalive window the system specification defines, rather than waiting indefinitely.
- **FR-005**: The application MUST verify at startup that the local SSH client is present and
  new enough for the connection-reuse path, and MUST refuse with a clear message when it is
  not.
- **FR-006**: The application MUST attempt authentication silently first, and MUST fall back to
  prompting only on an authentication failure — never on an unreachable host or a missing
  engine.
- **FR-007**: The application MUST render any passphrase prompt in its own interface, never in
  a terminal.
- **FR-008**: A passphrase MUST NOT be written to any log, diagnostic or crash report, and its
  buffer MUST be cleared after use.
- **FR-009**: The application MUST offer a credential picker when every automatic route has
  failed.
- **FR-010**: The application MUST frame every message on the link with an explicit length, as
  §4.1 defines, and MUST refuse a frame beyond the size limit with the error code that
  specification assigns.
- **FR-011**: The application MUST register a request's awaiting receiver before the request is
  written, so a reply cannot arrive before it can be matched.
- **FR-012**: Every request MUST have a time limit, after which it resolves as a failure and
  is removed from the registry. The default MUST be 30 seconds, and every call site MUST be
  able to set its own.
- **FR-013**: The registry MUST NOT retain entries for requests that have resolved, by any
  route.
- **FR-014**: A request MUST be withdrawable, and a withdrawn request MUST resolve rather than
  be silently dropped.
- **FR-015**: The application MUST classify a failure into exactly one of the conditions the
  system specification enumerates in §3.4, and MUST classify identically regardless of the
  system language.
- **FR-016**: A changed host identity MUST refuse the connection and warn the developer
  explicitly; forgetting a host identity MUST require explicit confirmation and MUST NOT
  happen automatically.
- **FR-017**: A missing engine MUST be reported as its own condition and handed to the feature
  that installs it, rather than being reported as a connection failure.
- **FR-018**: The project MUST provide a mock remote daemon that runs with no network access,
  can be driven into each failure condition on demand, and can simulate a link with delay and
  packet loss.
- **FR-019**: The transport's own tests MUST run against that mock, with no remote host and no
  network.
- **FR-020**: After a connection is lost, the transport MUST attempt to re-establish it
  without user action, with a growing interval between attempts, and MUST expose whether it
  is connected, retrying, or has stopped trying (`Disconnected`, which is reached only when
  the user stops it — the transport itself retries indefinitely). Requests outstanding when the connection died MUST
  resolve as failed rather than wait.
- **FR-021**: Outbound frames MUST be ordered by priority, with interactive traffic ahead of
  background work, as §4.6 requires. A large background payload already in flight MUST NOT
  delay an interactive request beyond one frame.

### Key Entities

- **Connection**: One authenticated link to one host, with a lifecycle — establishing,
  established, lost, torn down — that every logical channel shares.
- **Request**: One message expecting exactly one answer. Has an identifier unique for the
  session, a time limit, and exactly one of four outcomes: answered, failed, timed out,
  withdrawn.
- **Correlation registry**: The mapping from a request's identifier to whatever is waiting for
  its answer. Owns the guarantee that answers reach the right caller and that nothing
  accumulates.
- **Failure condition**: One of the enumerated reasons a connection or request failed, each
  with a response that follows from it. A failure the application cannot place in this set is
  itself a defect.
- **Mock daemon**: A stand-in for the remote engine that speaks the framing but no application
  protocol, and whose delay, loss and failure behaviour are set by the test.

## Success Criteria _(mandatory)_

### Measurable Outcomes

- **SC-001**: A developer connects to a remote host and reaches a usable connected state
  without opening a terminal, in 100% of the three credential situations in Story 2.
- **SC-002**: One authentication serves the whole session: across a session that opens
  multiple logical channels, the number of authentications is exactly one.
- **SC-003**: After the application quits, no process belonging to it remains, in 100% of
  trials.
- **SC-004**: With many requests in flight concurrently, every answer is delivered to the
  request that asked for it, with zero misdeliveries across a sustained run.
- **SC-005**: After a sustained run of many thousands of requests — answered, timed out and
  withdrawn — the number of retained entries for finished requests is zero.
- **SC-006**: A request that will never be answered stops occupying the connection within its
  time limit, in 100% of trials.
- **SC-007**: Each failure condition the system specification enumerates is reported as itself,
  and none is reported as a generic failure, in 100% of trials.
- **SC-008**: Failure classification is identical under a non-English system language, in 100%
  of trials.
- **SC-009**: A dropped network is reported as lost within the keepalive window rather than
  hanging, in 100% of trials.
- **SC-010**: The transport's full test suite runs to completion on a machine with no network
  access and no remote host.
- **SC-011**: Over a simulated link with 250 ms round-trip time and 5% packet loss, request
  exchange completes without lost or crossed replies, and the transport's added overhead —
  the time from handing a request to the transport until its answer is handed back, excluding
  the remote side's processing and the link's own round trip — stays at or below 15 ms at the
  99th percentile.
- **SC-012**: A connection dropped mid-session is re-established without user action once the
  host returns, in 100% of trials, and every request outstanding at the moment of the drop
  resolves rather than hanging.
- **SC-013**: With background traffic saturating the link, an interactive request is written
  ahead of the queued background work in 100% of trials.

## Assumptions

- **The transport mechanism is settled.** A-B1 decided it and §3.1 makes the invocation
  normative. This specification treats both as given. Reopening that choice is a change to the
  system specification, not to this feature.

- **A mock stands in for the remote engine, and the engine does not exist yet.** F002 puts it
  on the host. Everything here is verified against the mock, which speaks the framing and
  nothing above it. The risk this accepts is that the mock and the real engine could differ;
  the mitigation is that the mock implements only the framing layer, which is the layer the
  system specification defines normatively and exactly.

- **`[OPEN: NFR]` is not closed, and this specification does not close it.** The system
  specification's sub-250 ms budget names no percentile and no measurement point, and Appendix
  B, NFR records that. SC-011 therefore measures what this feature can be held to on its own:
  the transport's _added_ overhead over the link's round trip, rather than an end-to-end
  interaction time that depends on features not yet built.

  **Decided for this feature** (Clarifications, 2026-09-22), pending NFR: no more than 15 ms
  at the 99th percentile. A median was rejected because a transport's median is
  uninformative — queueing failures show up in the tail, and the tail is what a developer
  feels. This is a criterion F001 can be held to today; it does not define the product's
  budget, and if NFR closes on a different figure this changes with it.

- **Request time limits are per-request, not global** (Clarifications, 2026-09-22), with a
  30-second default that any call site may override. One global limit cannot serve both a
  completion that is stale after 250 ms and a build that runs for minutes. The default is
  deliberately generous: a limit that fires early turns a slow link into a broken one, and the
  interaction budget is protected by withdrawal (FR-014), not by timeouts.

- **Forgetting a changed host identity is offered here, not deferred.** §3.9 requires the
  action to exist and to be explicitly confirmed. It is in scope because the refusal it
  accompanies is in scope, and an explicit refusal with no route forward is a dead end that
  trains users to work around the application.

- **The remote side is assumed not to write diagnostics onto the error channel**, as §3.4
  makes normative for `ide-engine`. The mock honours that. If a real engine later violates it,
  the classification in Story 4 degrades — this is recorded as a known dependency on the
  engine's discipline rather than defended against here, because no framing on one channel can
  constrain another.

- **Reconnection lives here, the offline experience does not** (Clarifications, 2026-09-22).
  The transport owns the connection, so it owns recovery: it retries with a growing interval
  and reports whether it is connected, retrying or given up. F012 offline-readonly consumes
  that state and adds the read-only lock, cached-only behaviour and hash reconciliation on
  top. The alternative — detect here, recover in F012 — was rejected because F012 is four
  features downstream, and until it shipped a single network blip would end every session
  permanently, including for the features built in between.

  **This overlaps the feature map**, which currently gives F012 a subfeature reading
  "Connection state detection from keepalive expiry and pipe EOF". That detection is FR-004
  and FR-020 here. The map entry should be reworded to consume this feature's connection
  state rather than re-detect it; that edit is outside this specification's scope and is
  flagged rather than made.

- **The priority queue is built here** (Clarifications, 2026-09-22), even though only one
  class of traffic exists until prefetch and indexing arrive. §4.6 is normative and
  transport-level, no other feature in the map is assigned it, and an unassigned normative
  requirement is how a thing quietly never gets built. The second traffic class is exercised
  by the mock rather than by a real producer until F003.

- **Platforms too old to prompt inside the application degrade to the credential picker.**
  §3.3 notes the assisted attempt needs a newer SSH client than some supported distributions
  ship. Those users get the silent attempt and the picker, which is a worse experience but a
  working one.
