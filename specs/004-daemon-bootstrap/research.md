# Research: Daemon Bootstrap

**Branch**: `feature/F002-daemon-bootstrap` | **Date**: 2026-09-23 | **Plan**: [plan.md](./plan.md)

Decisions taken before implementation, per Constitution Principle III. Two of them bind features
beyond this one and are marked for promotion to Appendix A of the system specification — a
decision only this file records is one F003 will not find.

---

## Where the engine lives, and how the wire format is shared

**Decision**: The repository becomes a Cargo workspace with three crates: the existing client
(`src-tauri`), a new `engine`, and a new `protocol` crate holding the `Content-Length` framing
codec and the wire types both sides must agree on. F001's codec moves into `protocol` and the
client depends on it rather than owning it.

**Rationale**: §4.1 defines framing normatively and exactly. Two implementations of a normative
format is the precise failure the mock daemon was designed to avoid — the mock implements no
§4.8 method specifically so that it cannot drift from the engine. Letting the engine grow its
own codec would reintroduce that risk at the layer where it matters most, because a framing
disagreement corrupts every message rather than failing one.

This does not violate F001's rule that "nothing outside this directory may depend on the codec".
That rule keeps the *application layer* from reaching around its port, and it still holds: the
client's use cases see `RequestTransport`, not the codec. Sharing the wire format with the
process on the other end of the wire is a different relationship — it is the definition of a
protocol.

**Alternatives considered**: *Engine depends on the client crate* — drags Tauri, the window
controller and the session store onto a headless remote binary. *Duplicate the codec* — two
implementations of a normative format, rejected above. *Publish the protocol crate* — premature;
nothing outside this repository consumes it.

---

## How the binary reaches the remote host

**Decision**: Stream the artifact over `ssh` to a staged path on the remote host, writing it to
the child's stdin and letting the remote end redirect to a file. Promote it afterwards. The
invocation multiplexes over the control master F001 already established, so no second
authentication occurs.

**Rationale**: Three properties decided this, and only this mechanism has all three.

It needs no tooling on the remote host beyond a shell, which matters because the remote host is
whatever the developer can reach, not a machine we provisioned.

It reuses the connection that exists. A-B1 chose `ControlMaster` partly because it makes bulk
transfer cheap; this is the first feature to spend that.

It gives progress for free. SC-013 requires a report at least once per second with bytes
transferred and total — and only a mechanism we drive ourselves can count bytes as they go.
`sftp` and `scp` report progress as human-formatted lines on a terminal, which would have to be
scraped, and scraping a progress bar to satisfy a requirement is the kind of thing that works
until someone's OpenSSH formats it differently.

**Not the JSON-RPC channel.** At deployment time there is no engine, so there is no channel; and
§4.1 caps a frame at 1 MiB, which a binary measured in tens of megabytes would need chunking to
fit. Both facts point the same way.

**Alternatives considered**: *`sftp` in batch mode* — a real protocol rather than a shell
redirect, and its progress output is not machine-readable. *`scp`* — in OpenSSH 9 it is `sftp`
underneath, so it inherits the same problem with none of the benefit. *A download from a release
URL* — rejected at system level by A-BOOT, which requires no outbound internet on the host.

**→ Promote to Appendix A.** Every later feature that moves bulk data — F003's file reads,
F017's artifacts — needs to know that bulk goes beside the protocol channel over the control
master, not through it.

---

## How the client gets an engine binary to embed

**Decision**: The engine is built **before** the client, by the script layer that already
orchestrates builds, into a conventional path. `src-tauri/build.rs` reads that path — overridable
with `APEX_ENGINE_BIN` — embeds the bytes and computes the digest. If the artifact is absent,
the build fails with a message naming the step that was skipped.

**Rationale**: Cargo has no stable way to depend on another crate's *binary* artifact. Artifact
dependencies (`bindeps`) are nightly-only, and MSRV here is 1.75 stable. The obvious workaround —
having `build.rs` shell out to `cargo build -p engine` — invokes Cargo recursively while the
outer invocation holds the package lock, which deadlocks or races depending on version and
platform. It is the kind of thing that works on one machine and hangs in CI.

Ordering the two builds outside Cargo sidesteps the problem entirely, and this project already
has a place to do it: `npm run build` and the Tauri build already sequence steps today, so the
engine build is one more step rather than a new mechanism. No `xtask` crate, no build tool, no
nightly.

Failing loudly on a missing artifact matters more than it sounds. The alternative — embedding an
empty slice and discovering it at deployment — produces a client that ships, connects, deploys
zero bytes and fails verification against a host that did nothing wrong.

**Alternatives considered**: *Artifact dependencies* — exactly the right feature, and nightly.
*Recursive `cargo build` from the build script* — the lock problem above. *An `xtask` crate* —
the idiomatic Cargo answer to build orchestration, and a whole crate to introduce when a script
step already exists. *Check the binary into the repository* — a multi-megabyte artifact in git
that must be rebuilt by hand on every engine change.

---

## Which digest, and where it is computed

**Decision**: SHA-256. The client computes the digest of the artifact it ships at build time; the
remote side computes the digest of what landed using `sha256sum`, which is part of coreutils.
The two are compared before the artifact is made executable.

**Rationale**: The comparison has to happen where the file landed, which means the remote host
has to be able to compute it. That single constraint decides the algorithm: `sha256sum` is
present on every Linux host worth connecting to, and needs nothing deployed. A faster digest
would require deploying a tool to verify a deployment, which is circular.

Computing the shipped artifact's digest at build time, from the same file that gets embedded,
removes the possibility of the constant and the bytes disagreeing. A hand-maintained hash is a
hash that is eventually wrong.

**Alternatives considered**: *BLAKE3* — faster and a better hash, but not installed anywhere by
default, so verification would need a binary we have not yet deployed. *Comparing file size
only* — catches truncation and nothing else. *Verifying by running the binary with a version
flag* — executes the artifact to decide whether it is safe to execute, which is the wrong order.

---

## How a partially transferred artifact is kept out of reach

**Decision**: Transfer to a staged path in the same directory as the final one, verify, set the
executable bit, then `rename` into place. The staged name includes the digest, so concurrent
deployments of the same artifact converge and deployments of different artifacts cannot collide.

**Rationale**: `rename` within a filesystem is atomic, so no observer ever sees a partial file
under the final name. Staging in the *same directory* rather than a temp directory is what
guarantees the same filesystem; a cross-device rename fails, and a fallback copy would reopen
the window.

The executable bit is set only after verification, so a truncated transfer is not merely wrong
but unrunnable — the two defences are independent, which is what FR-006 asks for.

**Alternatives considered**: *Write directly to the final path* — leaves a window where a partial
binary is the engine. *Stage under `/tmp`* — likely a different filesystem, so the atomic rename
silently becomes a copy. *Lock file* — another state to clean up after a crash, solving a problem
`rename` already solves.

---

## What the client keeps when it replaces an engine

**Decision**: The replacement is staged and promoted under a **version-qualified name**, and the
previous engine's file is removed only after the replacement has completed a handshake. A stable
name points at whichever version is current.

**Rationale**: FR-021b requires the previous engine to survive until the new one has proven
itself, and verification is explicitly not proof — a binary can be exactly what was sent and
still fail to run here, which is the case the spec's edge list calls out. Only a completed
handshake demonstrates that.

Version-qualified names make this fall out of the layout rather than requiring a backup-and-
restore dance: the old engine is not "backed up", it is simply still there under its own name.
Rollback is then the absence of an action, which is what FR-021c asks for.

**Alternatives considered**: *Copy the old binary aside and restore on failure* — a restore path
that runs exactly when things are already going wrong, which is when it is least likely to have
been tested. *Keep every version* — a retention policy to design for a case that has not arisen;
A-OFFLINE's sibling question was answered the same way.

---

## Adding a restart notification to the protocol

**Decision**: Add `session/onRestart` to §4.8's Session group — a notification carrying the
session identity and a list of what the engine could not preserve. This requires an edit to the
system specification, which is made as part of this feature rather than assumed.

**Rationale**: FR-023 requires the engine to *tell* the client a restart happened rather than
leaving it to infer one, and §4.8 as written has no way to say it. `log/onMessage` is a log, and
a client that parsed log text to detect a restart would be depending on prose.

A-B6 already established that §4 is "the union of the source document's two versions plus the
methods required to make the described features work" — methods get added when a feature needs
one. This is that, done explicitly and recorded, rather than the engine inventing a method the
specification does not list.

**Alternatives considered**: *Re-run `auth/handshake` and let the client notice* — conflates
"the session restarted" with "a session is being established", so the client cannot tell a
reconnect from a restart, and FR-024c needs exactly that distinction. *A response flag on the
next request* — delays the news until the client happens to ask something, which may be never.

---

## When the protocol version increments

**Decision**: `protocolVersion` increments on a **breaking** change only. Adding a method,
adding an optional parameter, or adding a field to a result does not increment it. Removing or
renaming anything, changing a type, or making an optional parameter required does.

**Rationale**: Without this rule, adding `session/onRestart` in this very feature would bump the
version and make every engine in the field instantly incompatible — for a notification an older
client would simply ignore. A version that increments on additions is a version that forces a
redeployment for changes that needed none.

The rule is only safe because both sides are required to ignore what they do not recognise,
which is stated as a requirement here rather than left as an implementation habit.

**Alternatives considered**: *Increment on every change* — correct and useless, as above.
*Semantic versioning with major and minor* — more expressive, and A-BOOT deliberately made this
a single integer compared rather than negotiated; two numbers invite a compatibility matrix.

**→ Promote to Appendix A.** Every feature that adds a method needs to know whether it is
bumping a number that forces redeployment across the estate.

---

## How capabilities are expressed

**Decision**: A set of opaque string tokens, compared by exact match. The client offers a
feature when the engine's set contains the token that feature requires. Unknown tokens are
ignored by both sides.

**Rationale**: Exact-match tokens are the only scheme where "does this engine support X" has one
obvious answer. Ignoring unknown tokens is what makes the additive-change rule above work in
both directions.

**Alternatives considered**: *Structured capability objects with versions* — expressive, and
turns every capability check into a comparison with its own edge cases. *Infer capabilities from
`protocolVersion`* — collapses two independent axes, so an engine built without a feature
becomes indistinguishable from an older one.

---

## Building an engine for an architecture the developer does not have

**Decision**: Continuous integration builds the engine for every supported remote architecture
and the release client embeds them all. A local development build embeds only the host-native
engine, and the client refuses to deploy to an architecture it has no build for.

**Rationale**: The refusal is already FR-008, so a development build exercises a path that must
work anyway rather than a special case. Requiring every developer to install cross-compilation
toolchains to run the test suite would make the suite harder to run, and A-TEST's whole position
is that a suite which is hard to run stops being run.

**Alternatives considered**: *Require cross toolchains locally* — as above. *Fetch prebuilt
engines at client build time* — reintroduces a network dependency into the build and a second
trust root, which A-BOOT rejected for the same reason at deployment time. *Ship one architecture
and refuse the rest* — decides for users which instances they may use.

---

## Decisions deferred, with reasons

**How deployment progress is rendered.** The requirement is that progress is reported at least
once per second carrying bytes and total (FR-009); how the interface draws it is F000's design
system applied to a new state, and the shape follows the existing status bar work rather than
needing its own decision here.

**Whether the engine should daemonise.** It does not need to: the engine's lifetime is the SSH
channel's, and A-EC2 stops the whole instance when idle. Revisit if a session must outlive the
channel that created it — which A-OFFLINE's reconnection model may eventually want, but does not
require today.
