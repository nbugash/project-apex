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
cargo build -p apex-engine && cargo test --workspace
```

**Build the engine first.** Cargo does not rebuild another package's *binary* for a test that
merely executes it, so `cargo test` alone can run a current suite against a stale engine. That
is not hypothetical: it presented as every handshake test failing with `ConnectionLost`, a
symptom that says nothing about the cause. The harness now refuses to run against a binary older
than its sources and names the command to run — but building first avoids the refusal entirely.

The integration tests spawn the engine themselves. There is nothing else to start by hand.

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
cargo build -p apex-engine && cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

When reading `cargo test` output by script, count failures from each `test result:` line rather
than matching a field position — `test result: ok.` and `test result: FAILED.` put different
words in the same column, and a summary that checks the wrong one reports success over a failing
run. That mistake was made here once.

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

---

## Validation results

Run on 2026-09-23, Ubuntu, OpenSSH_9.6p1, Rust 1.75 target. Every scenario executed, not
inspected.

| Scenario | Command | Result |
| --- | --- | --- |
| 1. First connect to a bare host | `--test bootstrap_deploy` | 9 passed |
| 2. The engine says what it can do | `--test bootstrap_handshake` | 11 passed |
| 3. A newer engine is refused | `--test bootstrap_handshake` | included above |
| 4. Update and rollback | `--test bootstrap_restart` | 10 passed |
| 5. Session continuity | `--test bootstrap_restart` | included above |
| 6. Concurrency and atomicity | `--test bootstrap_deploy` | included above |
| 7. Deployment against a real `sshd` | `APEX_REAL_SSHD=1 --test bootstrap_real_sshd` | 4 passed |
| Workspace suite | `cargo build -p apex-engine && cargo test --workspace` | **274 passed, 0 failed** |
| Frontend | `npm run test:unit` | 30 passed |
| End to end | `xvfb-run -a npm run e2e` | 21/21 spec files, 65 screenshots under `linux/F002/` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| Format | `cargo fmt --all --check` | clean |

**SC-002, measured rather than asserted.** A-NFR requires the number, because a budget only ever
compared against tells nobody how much headroom is left:

```
SC-002: 8282040 bytes over 10 Mbit/s = 6.625632s transfer + 47.601µs overhead
      = 6.625679601s (budget 30s)
```

4.5x headroom. The client's own overhead is 47 microseconds — the budget is essentially all
transfer, which is the honest picture: this feature moves a binary, and nothing it does around
that is measurable beside it.

### What this run corrected

**The opt-in suite asserted on a banner that no longer exists.** It checked the deployed engine
printed `ide-engine` on startup — true of the placeholder, false of the real engine, which
prints nothing and blocks reading stdin. The test saw empty output and reported the binary had
not run. It now sends a handshake frame and asserts a framed reply, which proves the thing that
actually matters: the deployed artifact answers the protocol. Weakening the assertion would have
been the easy fix and the wrong one.

### What this feature does not prove

**That work continues while nobody is connected.** The engine's lifetime is its channel's: close
the connection and the process exits. The specification originally required the opposite, and
testing it directly showed the architecture could never have delivered it. Scope was narrowed and
**F020 `detached-engine`** carries the capability.

**That the ARM64 artifact runs.** A development build embeds only the host-native engine by
decision. CI builds both; a local build exercises the architecture-refusal path instead.

**That the engine serves a workspace.** It answers the handshake and owns session identity. It
implements no §4.8 workspace method — F003 adds those.
