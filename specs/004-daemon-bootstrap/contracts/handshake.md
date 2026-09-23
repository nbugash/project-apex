# Contract: Handshake and session

**Branch**: `feature/F002-daemon-bootstrap` | **Date**: 2026-09-23

The first exchange on a session, the compatibility rule, and what a session is allowed to
outlive. Entity shapes are in [data-model.md](../data-model.md).

This contract binds **both** implementations: the engine built by this feature, and any future
engine. §4.8 is the normative source for the method names; this document states what the
exchange guarantees.

---

## auth/handshake

**Guarantees**

1. **It comes first.** No other request is sent on a session before the handshake completes. An
   engine receiving anything else first may refuse it.

2. **Compatibility is a comparison, not a negotiation.** The client compares its
   `protocol_version` with the engine's and reaches exactly one of three verdicts. There is no
   common-subset path and no downgrade handshake.

3. **The client is the authority.** An older engine is replaced. A newer engine is refused, with
   no override — because a client that speaks a protocol it does not know produces confident
   wrong behaviour, which is worse than a refusal that names the problem.

4. **Capabilities are advertised, not discovered.** What the engine can do is stated in the
   handshake. The client offers exactly what the engine advertised, so a missing feature is
   invisible rather than broken.

5. **Unknown tokens are ignored on both sides.** This is what allows a method to be added
   without incrementing `protocol_version`, and it is required of both ends rather than left as
   a habit.

6. **A handshake that does not complete fails distinguishably.** "The engine never answered" is
   not reported as "the connection failed". They have different remedies, and F001 already
   proved the cost of blurring failure conditions.

**Does not guarantee**

- That every advertised capability works. Advertisement is a claim about intent, not a test
  result. A capability that is advertised and broken is a defect in the engine, not a violation
  of this contract.

---

## Resumption

A client that holds a `SessionId` from an earlier connection may present it to re-attach.

**Guarantees**

1. **A resumed session is the same session.** Work in progress continues; nothing is restarted
   because a client reconnected.

2. **A refused resumption is always visible.** If the engine does not recognise the identity, the
   response says so. The client MUST NOT treat a new session as a resumed one — a client that
   silently continues would show a developer work that is not happening, which is the worst
   available outcome because it looks like success.

3. **Resumption is not required for correctness.** A client that never resumes still works; it
   gets a new session each time. Resumption is what preserves in-progress work, not what makes
   the protocol function.

---

## session/onRestart

Sent by the engine after it re-executes itself. A notification: no id, no reply expected.

**Guarantees**

1. **A restart is announced, never inferred.** The client learns about it because it was told.

2. **The session identity is unchanged**, which is precisely what distinguishes a restart from a
   new session.

3. **What was lost is named.** `unpreserved` lists everything that did not survive, and an empty
   list is a positive assertion that nothing did. Omitting an item makes it appear to have
   survived, which is worse than reporting the loss — the developer would keep waiting for it.

**Does not guarantee**

- Delivery across an engine *crash*. A crashed engine sends nothing; the session is gone and the
  client discovers it when a resumption is refused. This is the limit recorded in the spec's
  clarifications, and it is why `unpreserved` covers re-execution rather than every restart.

---

## What this contract does not change

**A-REQ still holds.** An in-flight request dies with its connection, and a reconnecting client
re-issues whatever it still wants. Nothing here weakens that.

The distinction, which matters and is easy to lose: a *request* dies with the connection; the
*work* the engine had already started does not. A build keeps running while the laptop is shut.
A-REQ forbids pretending an unanswered request survived — it does not forbid the engine to
continue what it was already doing.
