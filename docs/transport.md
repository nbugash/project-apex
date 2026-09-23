# The transport

How the desktop client reaches the remote engine, and what a feature built on top of it can
rely on.

This is the working document. The binding statements live elsewhere and are not restated
here: the guarantees are in `specs/003-ssh-transport-core/contracts/transport.md`, the
decisions that bind later features are Appendix A `A-B1`, `A-REQ` and `A-PRI` in
`project-apex-predator.md`, and the invocation is §3.1.

## The shape of it

```
feature code
   │  RequestTransport (port)
   ▼
SshTransport ──────────► OpenSshSpawner ──► ssh ──► ide-engine
   │  registry              (§3.1 flags)              (remote)
   │  send queue
   │  frame codec
   ▼
ConnectionStatusSource (port) ──► status bar
```

Nothing above the port knows an `ssh` process exists. That is not decoration: the entire
test suite runs against a mock on the other side of the same port, and the composition root
binds the real transport or a stub without either use case noticing.

## What a caller can rely on

Five outcomes, exactly one of them, exactly once, for every request:

| Outcome | Means |
|---|---|
| `Answered` | The engine replied. The body is passed through uninterpreted. |
| `Failed { code, message }` | The engine replied with an error, or the request was refused before transmission. |
| `TimedOut` | No reply within the limit. Default 30 s; a caller that knows its own budget states a shorter one. |
| `Withdrawn` | The caller abandoned it. A cancellation was sent; the caller is released regardless. |
| `ConnectionLost` | The connection ended while it was in flight. |

`send` returns an outcome rather than a `Result`, because a failure is one of the five ways
a request ends and not an error to be distinguished from them.

`begin` is `send` for callers that may need to withdraw: it hands back the `RequestId`
alongside the pending outcome. By the time `send` returns there is nothing left to withdraw,
so a superseded completion request has to be started this way.

**An in-flight request dies with its connection** (`A-REQ`). Nothing is replayed. A save is
not complete until its response arrives, and must never be reported as successful on send.

**Priority is stated, not inferred** (`A-PRI`). The same method is interactive when the user
asked for it and background when a prefetch did. Bulk work sent as `Interactive` defeats the
guarantee for everyone.

## Connecting

Two phases, and §3.3 makes them mutually exclusive rather than merely ordered:

1. **Silent.** `BatchMode=yes`, which stops `ssh` blocking on a terminal prompt no GUI user
   can answer — and, as a consequence, disables `SSH_ASKPASS` entirely. An agent-held key
   connects here and the user sees nothing.
2. **Assisted.** No `BatchMode`; `SSH_ASKPASS` naming the bundled helper by absolute path,
   with `SSH_ASKPASS_REQUIRE=force`. Entered **only** from an `AuthenticationFailed`
   classification.

If both fail, the user is offered the identity picker rather than an error they cannot act
on. Below OpenSSH 8.4 the assisted phase is skipped entirely, because `SSH_ASKPASS_REQUIRE`
does not exist there and the prompt would be consulted or ignored depending on the platform
and on `DISPLAY`.

The passphrase travels app → unix socket → helper → pipe → `ssh`. The channel is *armed*,
never asking: it holds an answer the application already obtained, serves it once, and
answers a helper nobody armed it for with nothing.

## When it fails

| Condition | Retries | Response |
|---|---|---|
| `HostUnreachable` | yes | Backoff and try again. |
| `NetworkDropped` | yes | Backoff and try again. |
| `EngineCrashed` | yes | Backoff and try again. |
| `AuthenticationFailed` | no | The assisted phase, then the identity picker. |
| `EngineMissing` | no | Hand to the feature that installs the engine (F002). |
| `HostKeyChanged` | **never** | Refuse and warn. Forgetting the key is an explicit user action. |
| `Unknown` | no | Stop and report rather than guess a remedy that cannot work. |

Classification reads the exit code plus the last 8 KiB of stderr under `LC_ALL=C`, which the
invocation pins. The patterns are English because OpenSSH's C locale is English, not because
the user's machine is. When nothing matches, the answer is `Unknown` — a wrong
classification is worse than none, because it sends the user down a remedy that cannot work.

`HostKeyChanged` is checked before the authentication patterns, because it also exits 255. If
the generic patterns claimed it first, the user would get a passphrase prompt in response to
a possible attack.

## Reconnection

A loss is one observation: the child ended. `ssh` exits when its keepalive gives up, so there
is no second signal to wait for — which is why `ServerAliveInterval=15` and
`ServerAliveCountMax=3` are the most important flags in the invocation and the only ones no
integration test can check. Without them a pulled cable hangs forever instead of being
noticed in about 45 seconds.

On a loss: the queue closes, every outstanding request resolves as `ConnectionLost`, the
state goes `Disconnected`, and only then does a new attempt start. Backoff is 1 s doubling to
a 30 s ceiling with full jitter, retrying indefinitely. The ceiling exists so a laptop shut
for an hour reconnects within half a minute of waking; the jitter so every client of a
restarted bastion does not retry in lockstep.

The status bar shows `Retrying` with the attempt number and seconds remaining, because
"Reconnecting" with no sense of progress is indistinguishable from a hang.

## Testing against it

`tests/mock_daemon/README.md` documents the mock and its scripted behaviours. In short:
`MockSpawner::new("delay=250,drop=20")` gives you a slow, lossy link; `reorder=10` answers
out of order; `stall=...` models a link that has gone silent.

The default suite needs no network and asserts as much. One test is opt-in:

```sh
APEX_REAL_SSHD=1 cargo test --manifest-path client/core/Cargo.toml --test transport_real_sshd
```

It spawns a private `sshd` on loopback and carries frames over a real connection with the
real §3.1 option set. It is opt-in because it binds a socket, which the default suite must
not need — and it exists because no mock can tell you whether the installed `ssh` accepts
those options on this platform.

## Configuring a host

Not yet a feature. Until one exists, `APEX_REMOTE_HOST` and `APEX_REMOTE_USER` are read by
the composition root; without them the application runs unconnected against the stub rather
than failing to reach a host nobody named.
