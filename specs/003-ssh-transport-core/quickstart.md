# Quickstart: SSH Transport Core

**Branch**: `feature/F001-ssh-transport-core` | **Date**: 2026-09-22 | **Plan**: [plan.md](./plan.md)

How to run the transport and prove it works. Scenarios map to the user stories and success
criteria in [spec.md](./spec.md); entity shapes are in [data-model.md](./data-model.md) and
the port's guarantees in [contracts/transport.md](./contracts/transport.md).

Everything in the shell's quickstart still applies — see
[`../001-app-shell/quickstart.md`](../001-app-shell/quickstart.md).

---

## Prerequisites

| Requirement  | Notes                                                                                                                                                            |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rust 1.75+   | Already required by the shell                                                                                                                                    |
| OpenSSH 6.7+ | **Runtime** dependency, not linked. 6.7 is the floor for `ControlPath=%C` (§3.1); the assisted authentication path additionally needs 8.4+ and degrades below it |

**No remote host, no network, and no EC2 instance.** Every scenario below runs against the
mock daemon. That is a requirement (SC-010), not a convenience: a suite that needs
infrastructure is a suite that stops being run.

Check what you have:

```bash
ssh -V        # 6.7+ required, 8.4+ for in-app passphrase prompting
```

---

## Run

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

The transport's tests spawn the mock daemon themselves. There is nothing to start by hand.

---

## Validation scenarios

### 1. One connection, reused, torn down (User Story 1, SC-002, SC-003)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test transport_recovery
```

**Expected**: one authentication for the session however many channels open; after the app
exits, no `ssh` process remains. Confirm the second by hand once — the failure mode is an
orphaned master that outlives the app and holds the connection open:

```bash
pgrep -a ssh | grep apex- || echo "nothing left behind"
```

Use this form, not `pgrep -af "ssh .*apex"`: the `-f` variant matches the shell running the
check itself, so it always finds something and tells you nothing. That is not hypothetical —
it hid two real orphaned masters during this feature's own validation.

### 2. Replies reach the right requests (User Story 3, SC-004, SC-005)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test transport_exchange
```

**Expected**: with many requests in flight and replies deliberately reordered, every outcome
reaches its own request. After a sustained run of answered, timed-out and withdrawn requests,
the registry holds nothing (SC-005).

A registry leak will not fail a short test. The assertion is on the registry's size after the
run, not on the run completing.

### 3. Failure conditions are distinguished (User Story 4, SC-007, SC-008)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test transport_failures
```

**Expected**: each of the seven conditions in [data-model.md](./data-model.md) classifies as
itself, driven through `ScriptedSpawner` rather than a real `ssh`. Includes the same stderr
in a non-English locale, classifying identically (SC-008).

**Expected**: a changed host key does **not** retry. If it does, that is a security defect,
not a flaky test.

### 4. Loss, backoff, recovery (User Story 1, SC-009, SC-012)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test transport_recovery
```

**Expected**: the mock closes the pipe; every outstanding request resolves as
`ConnectionLost` rather than hanging; the supervisor retries with a growing interval; when the
mock accepts again, the connection re-establishes with no user action.

**Expected**: the interval grows. A tight retry loop passes a "did it reconnect" test and is
still wrong.

### 5. Interactive traffic goes first (SC-013)

Covered by `transport_exchange`.

**Expected**: with background requests saturating the queue, an interactive request is
written ahead of all queued background work. Not ahead of a frame already being written —
that is explicitly out of scope (research.md, "Head-of-line blocking within a frame"), and
SC-013 is worded to match.

### 6. Latency and loss (SC-011)

Covered by `transport_exchange`, against the mock's 250 ms / 5% profile.

**Expected**: no lost or crossed replies, and the transport's added overhead at or below
15 ms at the 99th percentile — measured from handing a request in to getting its answer back,
excluding the link's simulated round trip.

Measure added overhead, never wall-clock time. Wall clock is dominated by the 250 ms the
harness itself injects, and would pass regardless of what the transport does.

### 7. Hostile input does not corrupt the stream (Principle VI)

Covered by `transport_exchange`.

**Expected**: each row of the conformance table in
[contracts/framing.md](./contracts/framing.md) is survived, the stream stays aligned, and no
request in flight is affected.

Assert alignment by issuing a normal request **after** the hostile frame and seeing it
answered. A test that only asserts the bad frame was rejected would pass against a reader
that had silently desynchronised.

### 8. No credential reaches a log (FR-008)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test transport_failures -- passphrase_reaches
```

**Expected**: after a passphrase-assisted connect against the mock, the phrase appears in no
log, no error, and no panic payload. Asserted by searching captured output for a sentinel
passed as the passphrase.

---

## Automated suites

```bash
cargo test --manifest-path src-tauri/Cargo.toml        # all of the above
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
```

CI runs on manual dispatch only (`gh workflow run ci.yml --ref <branch>`), so nothing catches
a skipped check for you.

---

## What you cannot validate here

**That the real engine speaks this protocol.** The mock implements framing and nothing above
it; the engine does not exist yet (`[OPEN: H-BOOT]`, F002). The first real exchange happens
in F002, and the risk is bounded by the mock implementing only the layer §4.1 defines
normatively.

**That authentication works against a real host.** Every scenario uses the mock. The connect
_sequence_ is exercised; OpenSSH's actual negotiation with a real server is not, and cannot
be without a server.

Both are stated so nobody reads a green suite as proof of something it never tested.

---

## Validation results

Run on 2026-09-22, Ubuntu, OpenSSH_9.6p1, Rust 1.75 target. Every scenario below was executed,
not inspected.

| Scenario | Command | Result |
| --- | --- | --- |
| 1. One connection, reused, torn down | `--test transport_recovery` | 8 passed |
| 2. Replies reach the right requests | `--test transport_exchange` | 13 passed |
| 3. Failure conditions distinguished | `--test transport_failures` | 14 passed |
| 4. Loss, backoff, recovery | `--test transport_recovery` | included above |
| 5. Interactive traffic goes first | `--test transport_exchange` | included above |
| 6. Latency and loss | `--test transport_exchange` | included above |
| 7. Hostile input does not corrupt the stream | `--test transport_exchange` | included above |
| 8. No credential reaches a log | `--test transport_failures -- passphrase_reaches` | 1 passed |
| Unit tests | `--lib` | 139 passed |
| Lint | `clippy --all-targets -- -D warnings` | clean |
| Format | `fmt --check` | clean |
| Opt-in, real `sshd` | `APEX_REAL_SSHD=1 --test transport_real_sshd` | 2 passed |

**SC-011, measured rather than asserted.** The overhead test now prints what it measured, so
the headroom is on the record and not only the verdict:

```
SC-011 pure overhead: p50 37.129µs, p99 70.397µs (budget 15ms)
SC-011 beyond a 100ms round trip: p50 871.731µs, p99 1.132168ms (budget 15ms)
```

**SC-003, checked by hand as scenario 1 asks.** After the suite, `pgrep -a ssh | grep apex-`
finds nothing and no temporary directories remain.

### What this run corrected

Three things the validation found, all of them in the checks rather than the code:

1. **Scenario 8's filter matched the wrong tests.** `-- credential` selected two connect
   tests and not the redaction test at all. A filter that matches nothing — or the wrong
   thing — still reports `ok`.
2. **Scenario 7 had no integration coverage**, although this file claimed it did. The mock's
   `malformed` and `close-mid-frame` directives were exercised by no test. Three were added,
   each asserting that a *normal request afterwards* is answered, because "the bad frame was
   rejected" is equally true of a reader that silently desynchronised.
3. **The orphan check could not fail.** `pgrep -af` matched the shell running it. Corrected
   above, and it then immediately found two real orphaned control masters left by an earlier
   version of the opt-in `sshd` test — which appended its `ControlPath` override instead of
   prepending it, so `ssh` used the first value it was given and wrote a master into the
   developer's own `~/.ssh`, persisting for the hour `ControlPersist=1h` asks for.
