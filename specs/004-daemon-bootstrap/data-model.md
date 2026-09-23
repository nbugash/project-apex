# Data Model: Daemon Bootstrap

**Branch**: `feature/F002-daemon-bootstrap` | **Date**: 2026-09-23 | **Plan**: [plan.md](./plan.md)

Entities this feature introduces, their rules, and the two lifecycles that matter. Field-level
definitions live here and nowhere else; [design.md](./design.md) links to them rather than
restating them.

---

## EngineArtifact

What the client carries and deploys. One per supported architecture.

| Field | Type | Rule |
|---|---|---|
| `version` | engine version | The engine's own release version. Compared for display and logging, never for compatibility. |
| `protocol_version` | unsigned integer | What this engine speaks. The only field compatibility is decided by. |
| `architecture` | `Architecture` | The target this build runs on. |
| `digest` | `Digest` | Computed at build time from the embedded bytes. Never hand-written. |
| `bytes` | binary | The artifact itself, embedded in the client at build time. |

**Rules**

- An artifact's `digest` MUST be derived from its own `bytes` by the build, not recorded
  separately. A constant that can disagree with the file it describes eventually will.
- The client MUST hold at most one artifact per `architecture`. Two builds for one target with
  different digests is a build defect, not a runtime choice.

---

## Architecture

The remote target. A closed set, because the client can only carry builds it was built with.

| Value | Meaning |
|---|---|
| `LinuxX86_64` | The common case under A-EC2. |
| `LinuxAarch64` | Graviton instances. |

**Rules**

- An architecture the client carries no artifact for MUST be refused by name (FR-008). It MUST
  NOT fall back to another architecture, because a binary that cannot execute fails later and
  less clearly than a refusal.

---

## Digest

A SHA-256 hash, as lowercase hexadecimal.

**Rules**

- Comparison MUST be exact and case-insensitive on the hex, and MUST NOT be a prefix match. A
  prefix comparison is a weaker check that looks identical in passing tests.
- A digest computed on the remote host and a digest computed at build time are the same kind of
  value and MUST be compared as such — see [contracts/deployment.md](./contracts/deployment.md).

---

## DeploymentState

Where one deployment attempt has got to. Reported to the interface as it changes.

| State | Meaning |
|---|---|
| `Preparing` | Target architecture resolved, artifact selected, nothing transferred. |
| `Transferring { sent, total }` | Bytes are moving. Carries progress because FR-009 requires a report at least once per second, and a state without counts cannot satisfy it. |
| `Verifying` | Transfer complete, digest being computed on the remote host. |
| `Promoting` | Digest matched; the artifact is being made executable and moved into place. |
| `Complete` | The new artifact is in place and runnable. **Not** the end of a replacement: the previous engine is still present, and retiring it is a separate step the bootstrap use case takes once a handshake has proven the new one runs. |
| `Failed { reason }` | See `DeploymentFailure`. |

**Transitions**

```
Preparing ──→ Transferring ──→ Verifying ──→ Promoting ──→ Complete
    │               │               │             │
    └───────────────┴───────────────┴─────────────┴──→ Failed
```

**Rules**

- `Transferring` MUST be published at least once per second while it is current. A state that is
  entered and never re-published is indistinguishable from a stall, which is the failure FR-009
  exists to prevent.
- `Complete` MUST NOT retire a previous engine. The deployer cannot observe a handshake, so it
  cannot know whether the new artifact runs; a deployer that deleted the old engine on promotion
  would break the guarantee that a verified-but-unrunnable binary is survivable.
- No state after `Failed` exists. A failed deployment is retried as a new deployment, so that
  nothing carries state from an attempt that did not work.

---

## DeploymentFailure

Why a deployment did not complete. Each value exists because it needs a different response.

| Value | Cause | What the developer is told |
|---|---|---|
| `UnsupportedArchitecture { found }` | No artifact for that target. | Which architecture, and that this client cannot serve it. |
| `TransferInterrupted` | The connection dropped mid-transfer. | It can be retried. |
| `NoSpace` | The host has no room. | The host is full — not that the engine is broken. |
| `DigestMismatch` | What landed is not what was sent. | The deployment failed verification and will be retried. Explicitly **not** reported as a compromised host; the check cannot tell those apart, and the spec's threat model says so. |
| `NotExecutable` | Promotion succeeded but the file will not run. | The engine cannot run on this host. |
| `PermissionDenied` | The target directory is not writable. | Which path, so it can be fixed. |

---

## Handshake

The first exchange on a session. Both directions are part of the entity because neither half
means anything alone.

**Request** (client to engine)

| Field | Type | Rule |
|---|---|---|
| `client_version` | client version | Informational; never decides compatibility. |
| `protocol_version` | unsigned integer | What the client speaks. |
| `capabilities` | `CapabilitySet` | What the client can do. |
| `resume_session` | `SessionId`, optional | Present when re-attaching after a disconnection (FR-024b). Absent on a first connect. |

**Response** (engine to client)

| Field | Type | Rule |
|---|---|---|
| `engine_version` | engine version | Informational. |
| `protocol_version` | unsigned integer | What the engine speaks. Decides everything. |
| `capabilities` | `CapabilitySet` | What this engine can do. |
| `session_id` | `SessionId` | The session now in force. |
| `resumed` | boolean | True when `resume_session` was honoured. False means a new session was created — which the client MUST surface rather than treat as success (FR-024c). |

**Rules**

- The handshake MUST be the first request on a session, and no other request may be sent until
  it completes (FR-010).
- A handshake that does not complete within its limit MUST fail distinguishably from a transport
  failure (FR-014). "The engine never answered" and "the connection dropped" have different
  remedies.
- A response whose `resumed` is false when `resume_session` was sent MUST NOT be treated as a
  resumption. The work the client believed was running is gone.

---

## CapabilitySet

What one side can do, as a set of opaque string tokens compared by exact match.

**Rules**

- Unknown tokens MUST be ignored, not rejected. This is what lets a method be added without
  incrementing `protocol_version` — see research.md, "When the protocol version increments".
- The client MUST NOT send a request for a capability absent from the engine's set; such a
  request fails locally and the feature is presented as unavailable (FR-013).

---

## VersionVerdict

The comparison of two `protocol_version` values, and the whole of the compatibility decision.

| Value | Condition | Response |
|---|---|---|
| `Current` | Equal | Proceed. No deployment. |
| `EngineOlder` | Engine < client | Replace and re-execute, without involving the developer. |
| `EngineNewer` | Engine > client | Refuse the session. Tell the developer to update the client. Not overridable (FR-018). |

**Rules**

- The client is the authority (FR-015). This is a comparison, never a negotiation: there is no
  common-subset path, because that needs a compatibility matrix nobody maintains correctly.

---

## SessionId

An opaque identity for a session, minted by the engine.

**Rules**

- It MUST survive re-execution of the engine and loss of the connection (FR-024).
- It MUST NOT survive an engine crash. It lives in the engine's memory, not on disk — the
  deliberate limit recorded in the spec's clarifications.
- An identity the engine does not recognise MUST be reported as unrecognised rather than
  silently replaced (FR-024c).

---

## RestartNotice

What the engine sends after re-executing itself.

| Field | Type | Rule |
|---|---|---|
| `session_id` | `SessionId` | Unchanged across the restart, which is what makes it a restart rather than a new session. |
| `unpreserved` | list of descriptions | What did not survive. Empty is a meaningful value: it asserts that nothing was lost. |

**Rules**

- The engine MUST send this rather than leaving the client to infer a restart (FR-023).
- `unpreserved` MUST list everything that did not survive (FR-025). Omitting an item makes it
  appear to have survived, which is worse than reporting the loss.

---

## Session lifecycle

```
                    handshake
   [no session] ─────────────────→ Active
                                     │  │
             re-execution ───────────┘  │
             (identity preserved,       │
              RestartNotice sent)       │
                                        │
             disconnection ─────────────┤
             (session continues,        │
              work keeps running)       │
                                        ↓
                              engine crash / instance stop
                                        │
                                        ↓
                                   [no session]
```

**Rules**

- The session is owned by the engine and lives as long as the engine process (FR-024a). Work in
  progress continues while no client is attached.
- Re-execution preserves the session; a crash does not. Both are engine restarts from the
  outside, and the difference is visible to the client only because one sends a `RestartNotice`
  carrying the old identity and the other cannot.

---

## BootstrapOutcome

Where the whole sequence ended. The single value the use case returns.

| Value | Meaning |
|---|---|
| `Ready { session_id, capabilities, resumed }` | A session is established and usable. |
| `Deployed { session_id, capabilities }` | The same, after a deployment — distinguished so the interface can say what happened. |
| `RefusedNewerEngine { engine_protocol, client_protocol }` | FR-017. Carries both numbers so the message can name them. |
| `Failed { failure }` | A `DeploymentFailure`, or a handshake that did not complete. |
