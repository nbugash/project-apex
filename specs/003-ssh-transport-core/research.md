# Research: SSH Transport Core

**Branch**: `feature/F001-ssh-transport-core` | **Date**: 2026-09-22 | **Plan**: [plan.md](./plan.md)

Decisions whose blast radius is this feature. Decisions that bind other features belong in
Appendix A of the system specification — two below are marked for promotion.

The transport mechanism itself is **not** researched here. A-B1 decided it and §3.1 makes the
invocation normative; re-deriving it would create a second source of truth.

---

## Where the protocol's constants live

**Decision**: The framing rules, error codes and the 1 MiB cap are read from §4 of the system
specification and expressed **once** in code, in `domain/request.rs` and `domain/failure.rs`.
Neither `contracts/` nor this document restates the tables.

**Rationale**: §4 is normative and already complete. The F018 experience is the argument:
`ds-sync` read a value from the wrong place in the prototype and produced tokens that
disagreed with every approved screen, and nobody noticed because the numbers looked
plausible. A constant with two homes drifts; the only defence is one home.

**Alternatives considered**: Copying the error-code table into `contracts/` for reader
convenience — rejected, that is exactly the duplication Principle II forbids, and a reader
who needs the codes is one link away.

---

## Connection supervision and backoff

**Decision**: A supervisor task owns the child process. On loss it retries with exponential
backoff starting at 1 s, doubling to a 30 s ceiling, with full jitter. Retries continue
indefinitely; the user is told it is retrying and may stop it. Requests outstanding at the
moment of loss resolve immediately as failed — they are never carried across a reconnect.

**Rationale**: Clarified in the spec (2026-09-22): the transport owns the connection, so it
owns recovery. The ceiling exists because a laptop that has been shut for an hour should
reconnect within half a minute of waking, not back off to hours. Full jitter rather than
fixed doubling because every client of a restarted bastion would otherwise retry in lockstep.

Carrying requests across a reconnect was rejected deliberately: the remote engine has no
memory of a request issued on a dead connection, so a "resumed" request would wait forever
for a reply nobody will send. Failing fast and letting the caller decide is both honest and
simpler.

**Alternatives considered**: A fixed retry interval — simpler, but either hammers a down host
or reconnects slowly, and cannot be both. A bounded retry count — rejected because the
correct behaviour when a network is out for an hour is to still be trying.

**→ Promote to Appendix A.** Every feature that issues a request must know that an in-flight
request dies with the connection.

---

## Priority classification of outbound traffic

**Decision**: Two classes, `Interactive` and `Background`, with `Interactive` always ahead.
The class is a parameter of the send call, so the caller states it rather than the transport
inferring it. Within a class, order is FIFO.

**Rationale**: §4.6 is normative and assigns the ordering guarantee to the transport. Two
classes, not five: nothing in the system specification distinguishes more than "editor
traffic" from "background work", and a priority scheme finer than its requirements is a
scheme nobody can apply consistently.

The caller states the class because the transport cannot infer it — the same method
(`workspace/readFile`) is interactive when the user opens a file and background when prefetch
warms the cache. Inferring from the method name would be wrong in exactly the case that
matters.

**Alternatives considered**: Inferring priority from the method name — rejected above.
Strict FIFO until a second traffic class exists — rejected during clarification: §4.6 is
normative and unassigned, and an unassigned normative requirement is how a thing quietly
never gets built.

**→ Promote to Appendix A.** Every future caller must know which class its traffic is in.

---

## Head-of-line blocking within a frame

**Decision**: Priority ordering is applied between frames, not within one. A frame already
being written to the child's stdin completes before the next is chosen.

**Rationale**: A partially written frame cannot be interrupted without corrupting the
stream — `Content-Length` promises exactly that many bytes follow. The 1 MiB cap (§4.1) is
what bounds the resulting delay, and §4.6's other two rules (bulk moves to SFTP, large reads
are ranged) exist to keep frames far below it in practice.

This is why SC-013 says an interactive request must not be delayed "beyond one frame" rather
than "at all". The specification is honest about the bound rather than promising something
the wire format cannot deliver.

**Alternatives considered**: Chunking large frames so a high-priority frame can interleave —
rejected as a protocol change. §4.1 is normative and interleaving would require a framing
layer the engine does not implement.

---

## Testing failure classification without a real `ssh`

**Decision**: A `ProcessSpawner` port. `OpenSshSpawner` runs the real §3.1 invocation;
`ScriptedSpawner` produces a chosen exit code and chosen stderr text.

**Rationale**: The six conditions §3.4 enumerates, plus `Unknown` for anything that matches
none of them, must each be classified correctly and distinguished (SC-007),
including a changed host key and a missing engine. Provoking those from a real `ssh` on
demand requires a remote host that will refuse authentication, present a changed key, and
lack the engine — three hosts, or one host repeatedly reconfigured, in CI, over a network the
suite is required not to need (SC-010).

The port also keeps `LC_ALL=C` honest: the scripted spawner can emit localised stderr and the
test asserts classification is unchanged (SC-008).

**Alternatives considered**: Parsing stderr in a pure function tested directly, without a
port — simpler, but it tests the parser rather than the classification path, and the exit
code half of the rule would go unexercised.

---

## What the mock daemon is, and is not

**Decision**: A test-only binary speaking `Content-Length` framing and nothing above it. It
echoes, delays, drops, corrupts, stalls and closes on command. It implements no method from
the §4.8 catalogue.

**Rationale**: The mock exists so this feature is finishable before the engine exists. Giving
it application behaviour would make it a second implementation of the engine — which would
then drift from the real one, and whose divergence would be discovered by F002.

Restricting it to framing bounds that risk: framing is the layer §4.1 defines normatively and
exactly, so mock and engine can be checked against the same text.

**Alternatives considered**: Running the real `ide-engine` in tests — impossible, it does not
exist (`[OPEN: H-BOOT]`). A library-level fake with no process boundary — rejected, it would
not exercise the pipe, the framing codec, or the child-process lifecycle, which is most of
what this feature is.

---

## Latency and loss simulation

**Decision**: The mock applies delay and loss to whole frames at the application level, not
by shaping the kernel's network stack.

**Rationale**: SC-011 measures the transport's _added_ overhead, and SC-010 requires the
suite to run with no network at all. A pipe to a child process has no network to shape, and
the tests must run unprivileged in CI — `tc netem` needs `CAP_NET_ADMIN`.

Frame-level delay is a faithful enough model for what is being measured: queueing behaviour
and correlation under latency. It does not model TCP retransmission, and does not claim to.

**Alternatives considered**: `tc netem` or a SOCKS proxy — rejected on privilege and on
SC-010. A real network with an injected delay — rejected on both.

---

## Where the passphrase lives, and for how long

**Decision**: The passphrase exists in the app's memory only between the prompt returning and
the write to the helper's stdout. Buffer zeroed immediately after. Never logged, never in an
error message, never in a panic payload. Not cached: a second prompt asks again.

**Rationale**: FR-008 requires it. Not caching is the stricter choice and is deliberate — a
cache would need an expiry policy, a storage decision and a threat model, and OpenSSH's agent
already solves this properly for anyone who wants it. The right answer to "I do not want to
retype my passphrase" is `ssh-add`, not a cache we invent.

macOS Keychain storage is named in §3.7 for the identity-picker path; that is a platform
credential store, not a cache of our own, and is in scope only where §3.7 puts it.

**Alternatives considered**: An in-memory cache for the session — rejected above.

---

## Detecting a dead connection

**Decision**: The transport observes exactly **one** thing — the child process ending, which
arrives as EOF on its stdout. OpenSSH's keepalive (`ServerAliveInterval=15`,
`ServerAliveCountMax=3`) is not a second detector; it is what _bounds the time_ to that one
observation when a network dies silently, because exceeding it makes `ssh` itself exit.

So this feature has two obligations, and they are tested differently:

1. **Pass the flags.** The §3.1 invocation must carry them, or a pulled cable hangs forever.
   Unit-testable against the spawner: assert the invocation contains them.
2. **React to EOF promptly.** Integration-testable against the mock, which emulates `ssh` by
   going silent and then closing after the simulated window.

**Rationale**: An earlier version of this decision said "two signals, both required — EOF and
keepalive expiry", and claimed a pulled cable "produces no EOF". That is true only
momentarily. After the keepalive gives up, `ssh` exits and EOF follows; there is no second
detector for the transport to implement, and it has no socket to implement one on.

The distinction is not academic. The earlier wording produced a test that drove the mock to
stop answering _without_ closing the pipe and expected loss to be reported — which, with no
OpenSSH in the test path, would have waited forever. It described a mechanism this feature
does not own and cannot exercise.

**Alternatives considered**: A transport-level idle timer, so loss is detected without
relying on OpenSSH — rejected, it duplicates `ServerAliveInterval` at a layer that cannot see
the socket, and would fire spuriously on a legitimately slow reply. An application-level ping
over the pipe — rejected for the same reason, and it adds traffic to the pipe the interaction
budget protects.

---

## Stderr discipline and why classification can be trusted

**Decision**: Classification reads the child's stderr, which by §3.4 carries OpenSSH's
diagnostics only — the engine is forbidden from writing there. The transport nonetheless
bounds what it retains: the last 8 KiB, matched against a fixed pattern set.

**Rationale**: §3.4 makes the discipline normative but cannot enforce it, and this feature
cannot enforce it either — no framing on stdout constrains stderr. What it can do is fail
safely: a bounded buffer means a noisy engine degrades classification rather than exhausting
memory, and an unmatched pattern set yields "unknown failure" rather than a wrong
classification.

Recorded in the spec's Assumptions as a dependency on the engine's discipline.

**Alternatives considered**: Giving the engine a separate descriptor — rejected, it changes
the §3.1 invocation, which is normative.
