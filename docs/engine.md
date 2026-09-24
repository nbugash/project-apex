# The engine

`ide-engine` runs on the remote host and answers the client over the transport F001 built. F002
created it, deploys it, and establishes a compatible session with it.

Binding statements live elsewhere and are not restated: the deployment and version rules are
§3.8 and Appendix A `A-BOOT`, the method catalogue is §4.8, and the guarantees are in
`specs/004-daemon-bootstrap/contracts/`.

## How it gets there

The client **carries** the engine and pushes it over the SSH connection it already holds. No
package manager, no registry, no outbound internet on the remote host, no second credential.

```
client                          remote host
  │  ssh "uname -m"             │  which architecture?
  │  ssh "cat > .staged-<hash>" │  bytes, counted as they go
  │  ssh "sha256sum .staged-…"  │  verified where it landed
  │  ssh "chmod && mv"          │  executable only after verifying, then atomic
```

Four invocations, one authentication — they multiplex over the control master (`A-BULK`). Bulk
data never travels through the JSON-RPC channel: §4.1 caps a frame at 1 MiB, and at deployment
time there is no engine to talk to anyway.

Two properties are worth knowing because they are easy to lose:

**Staged and final share a directory.** `rename` is atomic only within a filesystem; across one
it silently becomes a copy, which reopens the window staging exists to close.

**The executable bit is set only after verification.** So a truncated transfer is both
unverified and unrunnable — two independent defences rather than one.

## Version compatibility

The client is the authority. `auth/handshake` exchanges `protocolVersion`, and the comparison
has exactly three outcomes:

| Engine | Response |
|---|---|
| Older | Replaced and re-executed. The developer is not asked. |
| Same | Proceed. |
| **Newer** | **Refused.** Update the client. No override exists. |

A newer engine is refused rather than attempted because speaking a protocol you do not know
produces confident wrong behaviour, which is worse than a clear refusal. There is no
common-subset path: that needs a compatibility matrix nobody maintains correctly.

`protocolVersion` increments on **breaking** changes only, and both ends ignore what they do not
recognise (`A-PROTOVER`). Adding a method is free.

## Replacement and rollback

A replacement is staged beside the engine it replaces, never over it, and the old one is
discarded only once the new one has **completed a handshake**.

Verification is not proof of runnability — a binary can be exactly what was sent and still fail
to start here. So rollback is the absence of an action: if the replacement never answers, the
previous engine is still in place and still serving.

## What a session outlives

| Event | Session survives? |
|---|---|
| Engine re-executes itself (`session/restart`) | **Yes** — `exec` keeps file descriptors, identity travels in the environment |
| Connection drops | No — the engine's lifetime is its channel's |
| Engine crashes | No — identity lives in memory, not on disk |

The middle row is a limit rather than an oversight, and it was found by testing rather than
reading: the specification originally required work to continue while nobody was connected, and
an engine spawned over `ssh` exits the instant its stdin closes. **F020 `detached-engine`**
carries that capability.

One consequence of re-execution that is easy to miss: `exec` keeps descriptors and discards
memory, so a request already read into the engine's buffer would vanish while the connection
stayed up. Anything buffered is refused with `-32000` and an instruction to re-issue, because
silence on a healthy connection is a lie.

## What it implements

Session methods only: `auth/handshake`, `session/shutdown`, `session/restart`, and the
`session/onRestart` notification. **No workspace method** — F003 adds those, and adds their
capability tokens at the same time.

The engine advertises only what it serves. A capability advertised and missing is a feature that
fails in a developer's hands, which is the failure capability exchange exists to prevent.

## Testing against it

The engine is a real binary spawned as a local child, so the suite needs no network:

```bash
cargo build -p apex-engine && cargo test --workspace
```

**Build the engine first.** Cargo does not rebuild another package's binary for a test that
merely executes it, so `cargo test` alone can run a current suite against a stale engine — which
presents as every handshake test failing with `ConnectionLost`, a symptom that says nothing
about the cause. The harness refuses a binary older than its sources and names the command.

The deployment path itself is opt-in, because it binds loopback:

```bash
APEX_REAL_SSHD=1 cargo test --test bootstrap_real_sshd
```

That suite is where the transfer, the remote `sha256sum` and the atomic rename meet a real
filesystem. It is worth its cost: it found a hang where the deployer became its own control
master and blocked forever reading its own output.

## Watching the workspace (F004)

The engine observes what the client asks it to and tells it what changed. Three pieces, and the
boundary between them is the whole design.

**The adapter** (`adapters/outbound/inotify_watcher.rs`) is the only file in the repository that
may name `inotify`; `engine/tests/inotify_confinement.rs` fails the build otherwise. It knows
about descriptors and kernel masks and nothing about what is worth reporting.

**The coalescer** (`application/coalescer.rs`) decides everything. It is pure — fed raw events
and told the time, opening no file and spawning no thread — which is what makes the volume
requirements arithmetic. "A thousand writes in one second yields at most ten events, and at
least one" runs instantly against a settable counter; the same assertion against a real clock
sleeps for a second, goes flaky under load, and gets marked ignored.

Its window is a **throttle, not a debounce**, and that distinction is the requirement rather
than an implementation detail. A deadline reset by every arrival would mean a file written
continuously reports nothing at all — bounded by elapsed time in the most useless possible
sense. The deadline is set once, by the first event for a path, and the last write's state is
what travels.

**The thread** (`adapters/outbound/watch_thread.rs`) owns the watcher, the coalescer and the
clock, and is reached by channel rather than by a mutex. `poll` blocks for as long as the
coalescer says its next window is, and a lock held across that would make every watch request
wait behind it — inside an interaction the developer initiated.

### The writer seam

Until F004 the engine had one writer, the stdio loop, so nothing had to coordinate. The watch
thread is the second. `adapters/outbound/frame_writer.rs` owns stdout and is taken for exactly
one frame: §4.6 makes this one pipe and one queue, and a frame interleaved with a reply is a
corrupt stream rather than a slow one. That lock is also what FR-016 measures — "event delivery
must not delay interactive traffic" is a claim about how long it is held.

### The exclusion set

Resolved once per workspace at `workspace/register` and stored **on the registered workspace**,
not inside the watcher. A-IGNORE requires one set shared by the watcher and the indexer, because
an indexer that indexes what the watcher ignores returns search results for files whose changes
are never noticed. Neither existed when this was written, so storing it there is what makes the
sharing structural rather than a convention somebody has to remember.

The `.gitignore` subset it understands is deliberately bounded and documented in
`application/exclusions.rs`. The `ignore` crate handles all of it and pulls `regex` with it, and
A-BOOT makes binary size a first-class concern on something transferred on every first connect.
A stated subset is a boundary; an unstated one is a bug waiting to be found.
