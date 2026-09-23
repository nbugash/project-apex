# Contract: Deployment

**Branch**: `feature/F002-daemon-bootstrap` | **Date**: 2026-09-23

What a caller may rely on when the client puts an engine on a host. Entity shapes are in
[data-model.md](../data-model.md); this document states guarantees, not fields.

---

## deploy

Place an artifact on a host and make it runnable.

**Caller provides**: the target host, and the architecture it runs.

**Guarantees**

1. **Nothing partial is ever executable.** At no point, under any interleaving of concurrent
   attempts or any failure, does a path that could be executed as the engine contain fewer bytes
   than the artifact. The staged name and the atomic rename are what deliver this; the
   executable bit being set only after verification is the independent second defence.

2. **What runs is what was sent.** The artifact is executed only after a digest computed on the
   remote host matches the digest computed from the embedded bytes at build time. A mismatch
   aborts, and nothing is executed.

   The property being checked is that the bytes arrived intact. It is **not** a defence against
   a hostile host, and a mismatch is reported as a failed deployment rather than as tampering —
   the check cannot tell those apart, and saying otherwise would be a guess. See the spec's
   threat model section.

3. **Progress is observable while it runs.** `Transferring` is published at least once per
   second with bytes sent and total. A caller can always distinguish a running transfer from a
   stalled one, which is the whole reason the state carries counts.

4. **An unsupported architecture is refused, not approximated.** No artifact is deployed to a
   target the client has no build for, and the refusal names the architecture found.

5. **It is idempotent.** Deploying an artifact already present with a matching digest transfers
   nothing and succeeds.

6. **The previous engine survives.** A deployment that replaces an existing engine leaves the
   previous one intact and executable until the replacement has completed a handshake. Failure
   at any point before that leaves the previous engine in place and running.

   This is stronger than "verify before promoting", deliberately. A binary can be exactly what
   was sent and still not run on this host, so verification is not proof of runnability and
   cannot be the thing that retires the old engine.

7. **Nothing requires elevated privilege.** Every path written is one the developer's own
   account owns.

**Does not guarantee**

- That the deployed engine will start. Only that it arrived intact and is executable. Whether it
  runs is answered by the handshake, and that answer is what promotion waits for.
- Any protection against a host the developer does not control. Out of scope by decision, not by
  oversight.

---

## retire_previous

Remove the engine a replacement superseded.

**Caller provides**: the version being retired.

**Guarantees**

1. **It is only ever called after a handshake succeeded** against the replacement. This is the
   caller's obligation, and it is why retirement is a separate operation rather than the last
   step of `deploy`: the deployer cannot observe a handshake, so it cannot be the component that
   decides the old engine is expendable.

2. **Failing to retire is not a failure of the update.** The new engine is running; an orphaned
   old binary costs disk and nothing else. It is reported and the session continues.

3. **It is idempotent.** Retiring something already gone succeeds.

---

## Failure reporting

Every failure resolves to one named `DeploymentFailure`, never a generic error. The set exists
because each member needs a different response from the developer: a full disk, an unwritable
directory, an unsupported architecture and a corrupt transfer are four different problems, and
collapsing them into "deployment failed" sends people to debug the wrong one.

`DigestMismatch` is retried as an ordinary failure. Repeated mismatches are reported, and are
still not called tampering.

---

## Concurrency

Two deployments of the same artifact to the same host converge: they stage under the same
digest-qualified name and the rename is idempotent. Two deployments of *different* artifacts
cannot collide, because the staged name differs.

No sequence of interleavings may produce an executable that is a mixture of two transfers. This
is asserted by test rather than argued, because "we thought about the interleavings" is not a
property anything can check.
