# Contract: RequestTransport

**Branch**: `feature/F001-ssh-transport-core` | **Date**: 2026-09-22

The port every later feature reaches the remote engine through. Stated as behaviour a caller
can rely on and an implementation must provide; signatures are in
[design.md](../design.md), entities in [data-model.md](../data-model.md).

Two implementations must satisfy this: the real OpenSSH-backed transport and the mock used in
tests. Anything a test can rely on from the mock must therefore be stated here, or the mock
becomes a second specification.

---

## send

Send a request and await its outcome.

**Caller provides**: a method name, parameters, a `Priority`, and optionally a time limit
(default 30 s).

**Returns**: exactly one `RequestOutcome`.

**Guarantees**:

1. **The reply reaches the caller that asked.** With any number of requests in flight and
   replies arriving in any order, an outcome is delivered to its own request and no other
   (SC-004). This is the guarantee the whole port exists for.
2. **Registration precedes transmission.** The awaiting receiver is installed before the
   frame is written (FR-011), so a reply arriving the instant the write completes is still
   matched.
3. **Exactly one outcome, exactly once.** No request resolves twice, and none resolves never.
4. **The deadline is honoured.** If no reply arrives in time, `TimedOut` — the caller is never
   left waiting past its own limit (SC-006).
5. **Nothing is retained afterwards.** However it resolved, the registry holds nothing for it
   (FR-013, SC-005).
6. **Priority is respected between frames.** An `Interactive` request is written before any
   `Background` frame not already being transmitted (FR-021, SC-013).

**Refuses**:

- A payload beyond the frame cap → `Failed` with the cap's error code (FR-010). Refused
  before transmission; an oversized frame is never partially written.
- A send while disconnected → `ConnectionLost` immediately, rather than queueing for a
  connection that may never return.

---

## withdraw

Abandon a request already in flight.

**Caller provides**: the `RequestId`.

**Guarantees**:

1. **It resolves.** The request completes as `Withdrawn` — never silently dropped (FR-014). A
   caller awaiting it is released.
2. **The remote side is told.** A cancellation notification is sent so the engine can stop
   work, per §4.5.
3. **Withdrawing an unknown or already-resolved id is not an error.** It does nothing. The
   caller races the reply by nature, and losing that race must not be a failure.

**Does not guarantee**: that the work stops remotely. §4.5 makes that best-effort. The caller
is released regardless.

---

## state / observe

Report the connection's state, and notify on change.

**Guarantees**:

1. **A current value is always available.** Never blocks waiting for a connection.
2. **A subscriber's view converges on the transport's.** Every change wakes subscribers, and
   the value they then read is the transport's current state — never a stale one.

   It does **not** guarantee that every intermediate state is delivered. Two changes in quick
   succession may coalesce to the latest, and that is the correct behaviour for the consumer
   this exists for: a status bar flashing "Connecting" for five milliseconds before
   "Connected" is noise, not information. An earlier draft of this contract promised "no
   state change is silently skipped", which the design cannot deliver and no caller needs;
   the test written against it failed, which is how the overpromise was found.
3. **`Retrying` carries progress.** Attempt number and next attempt time (data-model.md), so a
   caller can show that something is happening rather than a spinner that means nothing.

---

## Behaviour on connection loss

Stated separately because it is the contract's sharpest edge and every caller must know it.

1. **Every outstanding request resolves as `ConnectionLost`.** None waits for a link that no
   longer exists (FR-020).
2. **Recovery is automatic.** The transport re-establishes without caller action, backing off
   between attempts (research.md, "Connection supervision and backoff").
3. **No request is carried across.** A reconnect does not resend anything. The remote side has
   no memory of a request issued on a dead connection, so replaying it would wait forever.
   Retrying is the caller's decision, and `ConnectionLost` is distinguishable from `Failed`
   precisely so the caller can make it.

---

## What the mock must also satisfy

The mock is a `RequestTransport` too, so every guarantee above binds it. Additionally, for
tests to be meaningful:

- It must be drivable into each `FailureCondition` on demand (SC-007).
- It must apply configurable per-frame delay and loss (SC-011).
- It must **not** implement any §4.8 method. It frames, correlates and fails; it does not
  pretend to be the engine (research.md, "What the mock daemon is, and is not").

---

## Untrusted input

The child's stdout is untrusted (Principle VI). The implementation must survive, without
desynchronising the stream or exhausting memory:

- a declared length exceeding the cap,
- a declared length that never arrives,
- a body that is not valid JSON,
- a reply whose id matches nothing,
- a reply whose id matches something already resolved,
- two replies carrying the same id.

Each is refused or discarded on its own; none may affect a request still in flight. This is
the property that makes a hostile or broken engine a degraded session rather than a corrupted
one.
