# Quickstart: Daemon Bootstrap

**Branch**: `feature/F002-daemon-bootstrap` | **Date**: 2026-09-23 | **Plan**: [plan.md](./plan.md)

How to run the bootstrap and prove it works. Scenarios map to the user stories and success
criteria in [spec.md](./spec.md); entity shapes are in [data-model.md](./data-model.md) and the
guarantees in [contracts/](./contracts/).

F001's quickstart still applies — see [`../003-ssh-transport-core/quickstart.md`](../003-ssh-transport-core/quickstart.md).

---

## Prerequisites

| Requirement | Notes |
| --- | --- |
| Rust 1.75+ | Already required. This feature makes the repository a Cargo workspace, so `cargo test` at the root now builds the client, the engine and the shared protocol crate |
| OpenSSH 6.7+ | Runtime dependency, unchanged from F001 |
| `sha256sum` | On the **remote** host only, for verification. Part of coreutils; not needed locally |

**No remote host and no network.** Every scenario below runs against a locally spawned engine,
exactly as F001's suite runs against a locally spawned mock. That is a requirement (SC-010), not
a convenience.

The one exception is the opt-in `sshd` suite, which is where the deployment path itself is
exercised — see below.

---

## Run

```bash
cargo test                      # workspace root: client, engine, protocol
```

The integration tests spawn the engine themselves. There is nothing to start by hand.

---

## Validation scenarios

### 1. First connect to a host with no engine (User Story 1, SC-001, SC-002)

```bash
cargo test --test bootstrap_deploy
```

**Expected**: deployment runs to completion and a session is established, with no step the
developer had to take. A corrupted or truncated artifact is never executed.

**Expected**: progress is reported at least once per second while the transfer runs, carrying
bytes and total (SC-013).

Verify the second by asserting on the *number and spacing* of progress reports, not on their
existence. A single report at the start satisfies "progress was reported" while still looking
exactly like a hang, which is the failure the requirement exists to prevent.

### 2. The engine says what it can do (User Story 2, SC-008)

```bash
cargo test --test bootstrap_handshake
```

**Expected**: the handshake precedes every other request. A capability the engine did not
advertise produces **no frame on the wire** — assert on what was written, not on the error the
caller received, because a request that was sent and rejected also produces an error.

### 3. A newer engine is refused (User Story 3, SC-005)

Covered by `bootstrap_handshake`.

**Expected**: an engine reporting a higher protocol version ends the session with a refusal
naming both versions. No option, flag or retry proceeds anyway.

**Expected**: an older engine is replaced without the developer being asked.

### 4. Update and rollback (User Story 4, SC-006, SC-007)

```bash
cargo test --test bootstrap_restart
```

**Expected**: replacing an engine does not require the developer to reconnect.

**Expected**: a replacement that verifies correctly and then **fails to run** leaves the previous
engine in place and working. This is the case worth writing first — verification passing is not
proof of runnability, and a test that only corrupts the artifact never exercises it.

### 5. Session continuity (User Story 5, SC-009, SC-009a)

Covered by `bootstrap_restart`.

**Expected**: a restart is announced by the engine, never inferred by the client. The session
identity is unchanged, and anything that did not survive is named.

**Expected**: work in progress survives a disconnection and is still running when the client
re-attaches.

**Expected**: presenting an identity the engine has forgotten produces a stated refusal and a new
session — never a silent new session that looks like a resumption.

### 6. Concurrency and atomicity (SC-012)

Covered by `bootstrap_deploy`.

**Expected**: interleaved deployment attempts produce either one valid engine or a reported
failure, never a mixed artifact. Drive this with repeated concurrent attempts rather than a
single pair; one pair that happens to serialise proves nothing.

### 7. The deployment path against a real `sshd` (opt-in)

```bash
APEX_REAL_SSHD=1 cargo test --test bootstrap_real_sshd
```

**Expected**: an artifact is transferred over a real SSH connection, verified with the remote
host's own `sha256sum`, promoted, and executed.

Opt-in because it binds loopback, which the default suite must not need. It exists because the
local spawn cannot prove the one thing that matters most here: that the transfer, the remote
digest and the atomic promotion work against a real remote filesystem rather than a local one.

---

## Automated suites

```bash
cargo test                                             # all of the above
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

CI runs on manual dispatch only (`gh workflow run ci.yml --ref <branch>`), so nothing catches a
skipped check for you.

---

## What you cannot validate here

**That an engine deployed to a real EC2 instance serves a real workspace.** The engine built by
this feature answers the handshake and owns session identity. It implements no workspace method —
F003 adds those — so a successful bootstrap proves a session exists, not that anything useful
can be done in it.

**That the ARM64 artifact runs.** A development build embeds only the host-native engine, by
decision (research.md). Continuous integration builds both, and the architecture refusal path is
what a local build exercises instead.

Both are stated so nobody reads a green suite as proof of something it never tested.
