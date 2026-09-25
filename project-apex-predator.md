# Project Apex Predator — System Specification

**Status:** Active. This document is the source of truth for Project Apex Predator.
**Revision:** 2026-09-21.
**Supersedes:** the original architecture narrative (preserved at `project-apex-predator.md.bak`).

## How to read this document

Statements here are normative. Where a design decision was made deliberately, it is marked
**(A-xx)** and its rationale, alternatives and rejection reasons are recorded in Appendix A.
Where something is genuinely undecided it is marked `[OPEN: id]` and listed in Appendix B; those
markers are blocking for the feature they appear in. As of 2026-09-23 none remain open — Appendix
B records where each was answered, and Appendix A holds the decisions themselves.

Code in this document is normative only where a section says so. Everything else is
illustrative of intent, not of implementation.

---

# 1. Product Definition

## 1.1 What this is

A desktop IDE that separates a thin GUI client from a high-powered remote build and analysis
engine, communicating over SSH port 22 only. The client renders and captures input. The engine
holds the source of truth, runs language servers, compiles, tests and debugs.

The target instance is 16 vCPU / 128 GB RAM in `us-east-1`. The client is expected to run on
ordinary developer laptops.

## 1.2 Why the split exists

Local IDEs are bounded by the laptop. Indexing a large polyglot repository exhausts memory,
drains battery and produces multi-minute cold starts. Moving analysis to a 128 GB instance
removes that ceiling without asking the developer to change how they work.

The design constraint that makes this viable is network latency. Eastern Canada to `us-east-1`
is 15-35 ms RTT; Western Canada is 65-90+ ms. Both fit inside the interaction budget in §1.4,
but only if the client never waits on the network to render a keystroke.

## 1.3 Scope

**In scope:** macOS and Linux clients. Remote mode against a developer-owned EC2 instance.
Local mode against the developer's own machine. Seven language toolchains (§7.1).

**Out of scope:** Windows. No Win32-to-POSIX translation layer is written, no `cmd.exe`
handling, no `ReadDirectoryChangesW` path. This is a deliberate narrowing that lets the client
assume POSIX PTY semantics, native kernel file-watch APIs, and a single shared architecture
across both supported platforms.

**Out of scope for v1:** X11/VNC forwarding for native GUI applications produced by user code.
Recorded as a possible later extension in §9.4.

## 1.4 Interaction budget

The system targets **sub-250 ms** for any interaction a developer perceives as immediate.

These targets are measured at the 99th percentile, at the boundary between the interface and the
transport, excluding any delay a test harness itself injects, over at least 100 samples.
Cold-start and first-connect paths are excluded and budgeted separately, because deployment
(§3.8) and instance wake (§15.5) belong to a different question. A gate reports the measured
value, not only a verdict. See Appendix A, A-NFR.

| Property | Target | Notes |
|---|---|---|
| Keystroke to glyph | 0 ms network | Rendered from the local buffer; never awaits the engine |
| Completion request round trip | < 250 ms | Debounced 50-100 ms after typing stops |
| Sidebar folder expand (cached) | < 1 ms | Served from local SQLite |
| Sidebar folder expand (uncached) | < 250 ms | One shallow `workspace/readDirectory` |
| Engine availability | 99.9% | Excludes client network loss |
| Client resident memory | Low | Native webview, no bundled browser engine |

## 1.5 The three rules that protect the budget

1. **Local echo.** A keystroke enters the local buffer and renders immediately. The engine is
   informed asynchronously and is never in the path of rendering.
2. **Debounce and cancel.** Completion and diagnostic requests fire only after typing pauses.
   A newer request cancels the older one rather than queueing behind it.
3. **Connection reuse.** One authenticated SSH connection serves every logical channel.
   Nothing re-authenticates mid-session.

---

# 2. System Architecture

## 2.1 Topology

```
+------------------------------------------------------------------+
| CLIENT (macOS / Linux)                                           |
|                                                                  |
|  UI layer .......... Monaco (editor) + Xterm.js (consoles)       |
|         | async IPC                                              |
|  Core layer (Rust)                                               |
|    +-- Engine abstraction (Local | Remote provider)              |
|    +-- SQLite VFS cache (metadata, content blobs, git status)    |
|    +-- Transport: OpenSSH child process, JSON-RPC over stdio     |
+------------------------------------------------------------------+
                              |
                    Port 22, one connection
                              |
+------------------------------------------------------------------+
| REMOTE ENGINE (EC2, 16 vCPU / 128 GB)                            |
|                                                                  |
|  ide-engine (Rust, tokio) -- orchestrator                        |
|    +-- LSP multiplexer ...... jdtls, pyright/ruff, gopls,        |
|    |                          lexical, rust-analyzer, zls,       |
|    |                          clangd                             |
|    +-- DAP broker ........... delve, lldb-vscode, debugpy        |
|    +-- Process supervisor ... build/test/run, per-process limits |
|    +-- File watcher ......... inotify, scoped                    |
+------------------------------------------------------------------+
```

## 2.2 Division of responsibility

| Concern | Client | Engine |
|---|---|---|
| Rendering, input capture | Yes | No |
| Local text buffer, local echo | Yes | No |
| Syntax highlighting | Yes, Tree-sitter, cached locally | No |
| File tree projection | Yes, from SQLite | Source of truth |
| File content | Cached projection | Source of truth |
| Code intelligence | Relay only | Yes |
| Compilation, test, run, debug | No | Yes |
| Indexing | Never | Yes |
| Git computation | No | Yes |
| Git status projection | Yes, cached for colouring | Source of truth |

The client never indexes, never compiles and never holds authority over file content in remote
mode. Where the two disagree, the engine wins.

## 2.3 Engine abstraction

The UI issues provider-agnostic commands. The Rust core routes them to whichever provider is
active for the workspace, so the same frontend serves both modes.

```
        UI (Monaco, file tree, terminals)
                    |
            provider-agnostic commands
                    |
        +-----------+-----------+
        |                       |
  Remote provider         Local provider
  JSON-RPC over SSH       Direct POSIX syscalls
  Remote paths            Native paths
  Remote LSP daemons      Locally installed LSPs
```

Both providers implement the same trait (§6.1). Switching a workspace between modes swaps the
provider and re-points path mapping; it does not change the UI layer.

---

# 3. Transport and Connection

## 3.1 Decision (A-B1)

The client **spawns the system OpenSSH binary as a child process** and speaks length-prefixed
JSON-RPC over that child's stdin and stdout. No SSH library is linked into the client.

```
ssh -o ControlMaster=auto \
    -o ControlPath=~/.ssh/apex-%C \
    -o ControlPersist=1h \
    -o IPQoS=throughput \
    -o ServerAliveInterval=15 \
    -o ServerAliveCountMax=3 \
    -o BatchMode=yes \
    -o StrictHostKeyChecking=accept-new \
    <user>@<host> "/usr/local/bin/ide-engine --mode=pipe"
```

This invocation is normative. Each flag carries a reason:

- **`ControlMaster=auto` / `ControlPersist=1h`** establish a master connection that later
  invocations attach to without re-authenticating. This is what makes preview forwarding (§3.5)
  and bulk transfer (§3.6) cheap.
- **`ControlPath=~/.ssh/apex-%C`** uses the hashed form. macOS caps unix socket paths at 104
  bytes and the expanded `%r@%h:%p` form overflows it on long hostnames. Requires OpenSSH 6.7+.
- **`ServerAliveInterval=15` / `ServerAliveCountMax=3`** surface a dropped network as an error
  within ~45 s. Without them the pipe hangs indefinitely and the client never learns it is
  offline, leaving the reconnection loop (§11.4) with nothing to react to.
- **`IPQoS=throughput`** tunes socket behaviour for frequent small interactive packets.
- **`BatchMode=yes`** prevents `ssh` blocking on a tty prompt no GUI user can answer. It also
  disables `SSH_ASKPASS`, which is why connecting is a two-phase sequence (§3.3).
- **Compression is deliberately absent.** On a 15-35 ms link with ample bandwidth, per-frame
  compression adds CPU latency to the small interactive messages that dominate this workload.
  Enable it for bulk transfer only, after measurement.

The app MUST issue `ssh -O exit` on quit. `ControlPersist=1h` outlives the client process by
design, and without an explicit teardown the app orphans a master holding the connection open.

## 3.2 What this decision buys

Authentication is delegated entirely. The agent protocol, `~/.ssh/config` with all per-host
directives, `known_hosts` with hashed hosts and revocation, OpenSSH certificates, `ProxyJump`
for bastion hosts, FIDO `sk-` keys and PKCS#11 tokens all work because OpenSSH implements them.
No part of that surface is reimplemented here. See Appendix A, A-B1 for what was rejected.

## 3.3 Connect sequence

`BatchMode` and `SSH_ASKPASS` are mutually exclusive — BatchMode disables all interactive
querying, askpass included. Connecting is therefore two attempts:

1. **Silent.** Spawn with `BatchMode=yes`. Succeeds from the agent, or fails fast.
2. **Assisted.** On an *authentication* failure only, re-spawn without `BatchMode`, with
   `SSH_ASKPASS` pointing at the bundled helper and `SSH_ASKPASS_REQUIRE=force`. Requires
   OpenSSH 8.4+; skipped on older platforms (Ubuntu 20.04 ships 8.2).
3. **Fallback.** Anything unresolved raises the identity-file picker (§3.7).

### The askpass helper

`SSH_ASKPASS` points at a helper binary bundled in the app package. When OpenSSH executes it,
the helper opens a local IPC connection back to the running client, which renders the passphrase
prompt in the app's own UI and returns the passphrase on the helper's stdout.

Three requirements decide whether this works:

- `SSH_ASKPASS` MUST be an **absolute** path. OpenSSH execs the helper with an unpredictable
  working directory; a relative path fails precisely when a user needs the prompt.
- `SSH_ASKPASS_REQUIRE=force` MUST be set. Without it OpenSSH consults askpass only when it
  finds no tty, and behaviour varies by platform and `DISPLAY`.
- The passphrase crosses a process boundary through a pipe. It MUST NOT be logged, and the
  buffer MUST be zeroed after use.

## 3.4 Failure classification

The client spawns with `LC_ALL=C` so stderr text is stable across locales, then classifies on
exit code plus a bounded set of patterns:

| Class | Signal | Response |
|---|---|---|
| Host unreachable | exit 255, `Connection timed out` / `refused` | Offline mode, reconnect loop |
| Authentication failed | exit 255, `Permission denied` | Trigger assisted attempt, then picker |
| Host key changed | exit 255, `REMOTE HOST IDENTIFICATION HAS CHANGED` | Refuse connection, raise MITM banner |
| Daemon missing | exit 127 | Bootstrap flow (§3.8) |
| Daemon crashed | non-zero, not 255 | Restart with backoff, surface exit code |
| Network dropped mid-session | pipe EOF or keepalive expiry | Offline mode, reconnect loop |

**Daemon stderr discipline (normative).** The remote `ide-engine` inherits this same stderr
stream. If it logs there, its output interleaves with OpenSSH's diagnostics and corrupts the
table above. `ide-engine` MUST log to a file on the remote host or emit `log/onMessage`
notifications over the pipe. It MUST NOT write to stderr.

Note that `Content-Length` framing does not help here. Framing disambiguates messages within
*stdout*; stderr is a separate descriptor with two writers, and no framing discipline on one
stream constrains the other.

## 3.5 Preview forwarding (A-B2)

Local forwards are permitted, and exist solely so a developer can open a web application
running on the remote host in a local browser pane. LSP, DAP and file sync do **not** use
forwards; they ride the control pipe.

Forwards are added to and removed from the live master connection:

```
ssh -O forward -L 127.0.0.1:<local>:127.0.0.1:<remote> <user>@<host>
ssh -O cancel  -L 127.0.0.1:<local>:127.0.0.1:<remote> <user>@<host>
```

The local listener MUST bind `127.0.0.1` explicitly and MUST NOT bind `0.0.0.0`, which would
expose the remote service to the user's entire network. Ports are allocated dynamically per
preview window and released on close. Forwards MUST be torn down on disconnect, or the UI
presents a dead `localhost` port.

The earlier fixed-port design (5001/5002/5003, permanently open) is withdrawn. Static always-on
tunnels were the actual hazard; forwarding itself is not.

## 3.6 Bulk transfer

Large binary payloads — build artifacts, media, cloud-burst uploads — MUST NOT traverse the
control pipe. They use the SFTP subsystem attached to the same master connection:

```
sftp -o ControlPath=~/.ssh/apex-%C <user>@<host>
```

This attaches to the existing authenticated connection with no new handshake, and keeps the
control pipe reserved for small messages. See §4.6 for why this matters.

## 3.7 Identity fallback

If both connect attempts fail, the client presents an identity picker allowing the user to
select a key file directly (`id_ed25519`, `.pem`). On macOS, a passphrase the user elects to
remember is stored in the system Keychain, never in application preferences.

## 3.8 Daemon bootstrap and version negotiation

The client carries the engine binary and deploys it over the SSH connection it already holds,
into a per-user directory on the remote host. It does so when the engine is absent — which §3.4
classifies from `ssh` exiting 127 — and when the handshake reports a version older than the
client's. What landed is hashed against what the client shipped with before anything is
executed; a mismatch aborts without running it.

`auth/handshake` (§4.8) carries a `protocolVersion` integer that increments on any breaking
change to §4. The client is the authority: an older engine is redeployed and re-executed, and a
**newer** engine is refused with an instruction to update the client, because speaking a
protocol the client does not know produces confident wrong behaviour rather than an honest
failure.

This is what makes skew resolvable in one direction only, which is the property that keeps the
in-place replacement in §15.3 safe. See Appendix A, A-BOOT.

## 3.9 Host key policy

The client does **not** write to `~/.ssh/known_hosts`. `StrictHostKeyChecking=accept-new`
covers first contact. A *changed* key fails the connection, and the client surfaces OpenSSH's
own refusal as an explicit MITM warning.

Legitimate rotation — an instance re-provisioned with a new host key — is handled by a
user-confirmed "forget this host" action that removes the entry. This MUST NOT happen silently
or automatically; an IDE that trains users to dismiss host key warnings has removed the
protection entirely.

## 3.10 Preflight

At startup the client verifies `ssh` is present and reports its version. OpenSSH older than 6.7
(no `%C`) is refused with a clear message. Absence of `ssh` is refused likewise. These are
checked at startup rather than discovered at first connection failure.

---

# 4. Protocol Contract

This section is normative and is the single definition of the client-engine protocol. It
replaces the two partial and mutually inconsistent versions carried by the original narrative
(A-B6).

## 4.1 Framing

All traffic is JSON-RPC 2.0 over the stdio pipe, length-prefixed exactly as language servers
frame their traffic:

```
Content-Length: 184\r\n
\r\n
{"jsonrpc":"2.0","id":"req_fs_001","method":"workspace/readDirectory","params":{...}}
```

The reader consumes the header, allocates exactly that many bytes, and parses. A frame
exceeding **1 MiB** is a protocol error (`-32007`); payloads that large belong on the SFTP
channel (§3.6).

## 4.2 Message types

- **Request** — carries `id`, expects exactly one response.
- **Response** — carries the matching `id` and either `result` or `error`.
- **Notification** — carries no `id`, expects no response. Used for streams: process output,
  file events, git status, log lines.

Every request `id` is unique for the life of the session.

## 4.3 Correlation

The transport task owns a `HashMap<RequestId, oneshot::Sender<Response>>`. A request registers
its receiver **before** the frame is written, so a reply cannot arrive before the registry can
resolve it. This registry is the core of the transport and is the first component to build.

A request with no reply within its timeout resolves as `-32603` and is removed from the
registry. Registry entries MUST NOT accumulate for dead requests.

## 4.4 Errors

Standard JSON-RPC codes apply (`-32700` parse, `-32600` invalid request, `-32601` method not
found, `-32602` invalid params, `-32603` internal). Application codes occupy `-32000`..`-32099`:

| Code | Meaning |
|---|---|
| -32001 | Workspace not found or not registered |
| -32002 | Path escapes workspace root — refused (§4.7) |
| -32003 | File or directory not found |
| -32004 | Write conflict: `baseSha256` does not match current content |
| -32005 | Language server unavailable for the requested language |
| -32006 | Task not found |
| -32007 | Payload exceeds the frame limit |
| -32008 | Request cancelled by the client |
| -32009 | Workspace root no longer exists — registered, but the directory is gone |
| -32010 | Task identity is already running — refused rather than starting a second process |
| -32011 | Command could not be started — not found, not executable, or `cwd` unusable |

Every error carries a human-readable `message`. Errors that a user can act on carry a `data`
object describing the remedy.

`-32006` means the identity is unknown, and **not** that the task has finished. It formerly read
"Task not found or already exited", which contradicted two requirements at once: stopping a task
that has already stopped is a success, since the caller asked for it not to be running and it is
not running, and a client reattaching to a task that finished while it was disconnected is
entitled to learn how it finished rather than be told the task never existed. An implementation
following the old wording literally would have failed both and passed review, because the wording
was the specification.

`-32010` and `-32011` exist because two refusals a client must tell apart had no way to be told
apart. `-32010` says the identity is live — the correct response is to attach, not to retry — and
without it the only candidate was `-32006`, whose meaning is the exact opposite. `-32011` says the
command itself could not be started, which is the developer's mistake to fix and not the engine's
failure; routing it through `-32003` would borrow a code reserved for paths inside a workspace to
describe a program name resolved against `PATH`, which is not a workspace path at all.

`-32009` is deliberately distinct from `-32001`, because the two demand **opposite** responses.
`-32001` means the engine has never been told about this workspace and the client should register
it — which is also how a client recovers after an engine restart. `-32009` means the workspace was
registered and the thing it pointed at has been deleted, and the client must tell the developer and
stop presenting its cached projection as a live view. Routing a deleted workspace through `-32001`
would send the client into a re-registration that then fails because the root is no longer a
directory, surfacing a registration error for a deletion.

## 4.5 Cancellation

`$/cancelRequest` with `{ "id": "<in-flight id>" }` is a notification. The engine makes a best
effort to abandon the work and MUST still emit a response for the cancelled request, carrying
`-32008`. This exists because the interaction budget (§1.5, rule 2) requires a superseded
completion request to stop consuming the pipe, not merely be ignored on arrival.

## 4.6 Stream discipline

One pipe is one queue, so a large response serialises ahead of everything behind it. A
multi-megabyte file read would otherwise block a pending completion request and breach the
interaction budget the architecture exists to protect. Three rules prevent this:

1. Frames are capped at 1 MiB (§4.1).
2. Bulk payloads move to SFTP (§3.6), never the control pipe.
3. Reads of large files use the ranged form of `workspace/readFile` so the client fetches
   what it displays and streams the remainder as the user scrolls.

**Interactive traffic wins the race to the wire, in both directions.** The requirement is that
ordering, not any particular mechanism for achieving it, and the rule is stated per direction
because stating it once left half of it unbuilt. Client to engine: editor and LSP requests ahead
of background work. Engine to client: LSP responses, file events and command replies ahead of
bulk output such as indexing status and a task's stdout.

The two directions are built differently, and deliberately so. The client queues, because it
composes frames faster than the link drains them and a queue is what lets it reorder work that
already exists — `client/core`'s send queue. The engine does **not** queue. Its writer holds one
lock for the duration of a frame, and a producer blocked on that lock is a producer that has
stopped producing: the reader thread stops reading the pseudo-terminal, its buffer fills, and the
task blocks in `write`. That chain is how a task is slowed rather than truncated (§7.3), and it
exists only because nothing buffers between the producer and the wire.

So the engine grants priority by **making a bulk producer wait its turn**, not by queueing its
output. A writer with interactive traffic to send registers that fact; a bulk writer yields while
any such writer is waiting. Interactive frames therefore reach the lock first while bulk output
stays exactly as blocking as it was, which is what keeps one mechanism from paying for the other.

This was previously specified as a queue on both sides. A queue in the engine removes the
blocking that the slowing depends on, and separating a task's output from its exit into two
priority classes lets the exit overtake the output it was meant to follow. Both are properties
the blocking writer provided for free and neither was written down until something removed them.
A bulk producer starved by sustained interactive traffic is admitted and bounded: after a stated
number of consecutive yields one bulk frame goes through, so priority is a strong preference and
never a monopoly.

## 4.7 Path safety

`relativePath` in every method is relative to the workspace root and is **untrusted input**.
The engine MUST canonicalise the resolved path and assert it is a descendant of the workspace
root, rejecting anything else with `-32002`. Symlinks that resolve outside the root are
rejected on the same basis.

This is enforced on both sides of the link. The engine runs as an ordinary user with full
filesystem rights, so a malformed or hostile frame must not be able to read or write outside
the workspace.

## 4.8 Method catalogue

### Session

| Method | Kind | Params | Result |
|---|---|---|---|
| `auth/handshake` | request | `clientVersion`, `protocolVersion`, `capabilities`, `resumeSession?` | `engineVersion`, `protocolVersion`, `capabilities`, `sessionId`, `resumed` |
| `session/shutdown` | request | — | — |
| `session/restart` | request | — | — |
| `session/onRestart` | notification | `sessionId`, `unpreserved[]` | — |
| `log/onMessage` | notification | `level`, `message`, `source` | — |

`protocolVersion` is an integer that increments on a **breaking** change to this section only.
Adding a method, adding an optional parameter or adding a field to a result does not increment
it; removing or renaming anything, changing a type, or making an optional parameter required
does. Both ends MUST ignore what they do not recognise, which is what makes that rule safe — and
without it, adding `session/onRestart` would have forced a redeployment across every host for a
notification an older client would simply have ignored.

On mismatch the client is the authority: it redeploys an older engine, and refuses a newer one
rather than guessing at a protocol it does not know (§3.8, Appendix A, A-BOOT).

`auth/handshake` carries `resumeSession` when a client is re-attaching after a disconnection,
and the response's `resumed` says whether that was honoured. A false `resumed` means a **new**
session was created, and the client MUST surface that rather than treat it as success — a client
that silently continues shows a developer work that is not happening.

`session/restart` asks the engine to replace its own process image (§15.3). It is acknowledged
before the replacement happens, because afterwards there is no process left to answer with.

**Anything the engine has already read but not yet answered is refused with `-32000` before the
replacement**, rather than disappearing. Replacing a process keeps its file descriptors and
discards its memory, so a request that arrived moments earlier is gone while the connection
stays up — and the client would wait for a reply that can never come. A-REQ permits losing a
request when the connection dies; here the connection survives, so silence would be a lie. A
client receiving `-32000` re-issues.

`session/onRestart` is sent by the engine after it re-executes itself. The session identity is
unchanged, which is what distinguishes a restart from a new session, and `unpreserved` names
everything that did not survive. An empty list is a positive assertion that nothing was lost, not
an absence of information. A crashed engine sends nothing; its session is gone, and the client
discovers that when a resumption is refused.

### Workspace

| Method | Kind | Params | Result |
|---|---|---|---|
| `workspace/register` | request | `workspaceId`, `path` | `{name, canonicalPath}` |
| `workspace/close` | request | `workspaceId` | — |
| `workspace/readDirectory` | request | `workspaceId`, `relativePath`, `cursor?`, `limit?` | `items[]` of `{name, type, size, modified}`, `nextCursor?` |
| `workspace/stat` | request | `workspaceId`, `relativePath` | `{type, size, modified, sha256}` |
| `workspace/readFile` | request | `workspaceId`, `relativePath`, `offset?`, `length?` | `{content, encoding, sha256, totalSize}` |
| `workspace/writeFile` | request | `workspaceId`, `relativePath`, `content`, `baseSha256` | `{sha256}` |
| `workspace/createFile` | request | `workspaceId`, `relativePath` | `{sha256}` |
| `workspace/createDirectory` | request | `workspaceId`, `relativePath` | — |
| `workspace/rename` | request | `workspaceId`, `fromPath`, `toPath` | — |
| `workspace/delete` | request | `workspaceId`, `relativePath`, `recursive` | — |
| `workspace/search` | request | `workspaceId`, `query`, `maxResults` | `matches[]` |
| `workspace/watch` | request | `workspaceId`, `paths[]` | `{watching, refused[]}` |
| `workspace/unwatch` | request | `workspaceId`, `paths[]` | `{watching}` |
| `workspace/onFileEvent` | notification | `workspaceId`, `events[]` of `{event, relativePath, toPath?, type?, size?, modified?}` | — |
| `workspace/invalidateAll` | notification | `workspaceId` | — |

`workspaceId` is mandatory on every workspace method. The original "formal" contract omitted
it, which silently removed multi-workspace addressing.

`workspace/watch` and `workspace/unwatch` exist because watching is scoped to what the developer
has open (A-WATCHSCOPE), and the engine cannot infer that. Until they were added the catalogue had
two file-event notifications and no way to begin or end a watch, while §6.1 declared `watch()` on
the provider trait — the fifth absence of this kind, and the same shape as `workspace/register`
below. Both take a **list** and both are idempotent against a set the engine holds per workspace,
which is what lets a reconnecting client re-establish everything with one call rather than
replaying a remembered history.

The paths in that list are **what the client cares about, not what the engine will watch**: a
folder path for an expanded folder, a **file** path for an open editor tab. The engine derives the
directories to watch from it — the folders, the parent of each named file, their ancestors, and
the root. The distinction matters because a folder holding an open file would otherwise arrive as
one path for two reasons, and unwatching on a collapse could not be told from unwatching on a tab
close; the client would stop being told about a file it still has open. `refused[]` carries the
paths the host could not watch, so exhausted capacity is reported rather than silently producing a
watcher that delivers nothing (§10.3).

`workspace/onFileEvent` carries an **array**. The engine coalesces before it emits (A-COALESCE),
so everything whose window closed together travels in one frame; one pipe is one queue (§4.6), and
a burst delivered as hundreds of separate frames would take the writer hundreds of times ahead of
whatever interactive request is behind it. `event` is one of `created`, `modified`, `deleted` or
`renamed`; `renamed` is the only kind that sets `toPath`, and it is emitted once for a directory
however large the subtree beneath it, the client rewriting descendant paths in its own projection.
`created` and `modified` carry `type`, `size` and `modified` — the same entry metadata
`workspace/readDirectory` returns, and for the same reason: without them a newly created file
cannot be placed in the tree at all, and the client would have to ask about a path it was just
told about. This is metadata, not content: no event ever carries bytes, and no event makes cached
content valid, which remains a hash comparison and nothing else (§5.3).

`workspace/register` tells the engine what a `workspaceId` means. Until this was added the
catalogue presumed it in two places and defined it nowhere — §15.4 step 3 says to register a
workspace with the engine, and `-32001` below is reserved for one "not registered" — which left
every other workspace method unusable as specified. The engine canonicalises the root once here,
so each later request is a resolve and a prefix comparison rather than a second canonicalisation.
Registering an id that is already registered against the same path succeeds and changes nothing;
against a **different** path it is an error, because two meanings for one identity is precisely
what `workspaceId` exists to prevent. The registry is in memory and dies with the engine, so a
client re-registers after a restart — `session/onRestart`'s `unpreserved` list is how it learns it
must.

`workspace/close` is the counterpart `workspace/register` never had. Closing a workspace must stop
the tasks belonging to it and release its watches, and until this row existed the catalogue had no
frame meaning "I am finished with this workspace" — leaving that obligation stated in §7.3 and
unreachable through the protocol. It is deliberately **not** the same event as a dropped
connection: under A-TASKLIFE a connection that drops leaves tasks running, because a laptop moving
between networks must not kill a build, whereas closing the workspace is the developer saying they
are done with it. Conflating the two would make the protocol unable to express the difference
between an accident and an intention.

`workspace/readDirectory` is **paged**. `limit` defaults to and is capped at 1000 entries, and
`nextCursor` is present exactly when more entries follow. Entries are ordered
`(type DESC, name ASC)` — directories first, then by name, byte-wise on the UTF-8 encoding — and
**that ordering is part of the contract**, because the cursor is derived from it. A page request
therefore needs no server-side iterator, survives a restart, and can never duplicate or skip a
stable entry the way an offset would. Changing the ordering later is a breaking change and would
increment `protocolVersion`; adding the three optional fields did not.

`nextCursor` is an **opaque token**, not a bare filename, and a client MUST treat it as opaque:
pass back what was received and compare it against nothing. It encodes the whole ordering key,
because encoding only the name is wrong the moment a directory and a file interleave — with
directories `a` and `z` and a file `b` the listing is `a, z, b`, and a name-only cursor resuming
after `z` finds no later *name* and drops `b` entirely. That defect is invisible to any test whose
fixture holds entries of a single type, which is how it survived until an implementation exercised
a mixed directory.

`encoding` is `utf8` or `base64`. Binary files are legal and are returned base64-encoded within
the frame limit, or fetched over SFTP when larger.

**Field names in the tables above are written camelCase for readability; the wire carries
snake_case.** `workspaceId` is `workspace_id` in a frame, `relativePath` is `relative_path`,
`nextCursor` is `next_cursor`. The two ends share one definition of these messages in the
`protocol` crate, so they cannot disagree with each other — but a third party implementing this
protocol from the tables alone would send names the engine does not recognise, which is why the
mapping is stated rather than left to be inferred. F002 established the convention with
`clientVersion` as `client_version`; this section now says so.

`writeFile` carries `baseSha256` — the hash the client believed current when it began editing.
The engine rejects a mismatch with `-32004` rather than overwriting. A write is not complete
until its response arrives; the client MUST NOT report a save as successful on send.

### Language intelligence

| Method | Kind | Params | Result |
|---|---|---|---|
| `lsp/request` | request | `workspaceId`, `language`, `payload` | `payload` |
| `lsp/notify` | notification | `workspaceId`, `language`, `payload` | — |
| `lsp/onNotification` | notification | `workspaceId`, `language`, `payload` | — |
| `lsp/onServerState` | notification | `workspaceId`, `language`, `state` | — |

`payload` is a complete, unmodified LSP message. The engine is an envelope and a router, not a
translator. `state` is one of `starting`, `ready`, `crashed`, `restarting`, `unavailable`, and
exists so the UI can tell "no completions because the server died" from "no completions here".

### Execution

| Method | Kind | Params | Result |
|---|---|---|---|
| `execution/runTask` | request | `workspaceId`, `taskId`, `command`, `cwd?`, `env?`, `pty`, `cols?`, `rows?` | `{pid}` |
| `execution/attach` | request | `workspaceId`, `taskId` | `{pid, running, retained, exitCode?, signal?}` |
| `execution/list` | request | `workspaceId?` | `{tasks[]}` |
| `execution/writeStdin` | notification | `taskId`, `data` | — |
| `execution/resizePty` | notification | `taskId`, `cols`, `rows` | — |
| `execution/terminate` | request | `taskId`, `signal` | — |
| `execution/onStdout` | notification | `taskId`, `data` | — |
| `execution/onStderr` | notification | `taskId`, `data` | — |
| `execution/onExit` | notification | `taskId`, `exitCode?`, `signal?` | — |

`writeStdin`, `resizePty`, `terminate` and `onExit` did not exist in the original contract,
which made the integrated terminal write-only and left no way to stop a runaway process or
learn that a build finished.

`execution/attach` exists because a task outlives the connection that started it (A-TASKLIFE). A
client that reconnects, or that restarted, needs to reach a task it did not start in this session,
and `runTask` starts one rather than finding one. Attaching is deliberately a **different call**
from starting: a client racing its own reconnection must not silently start a second build under
an identity that already has one, and an idempotent `runTask` would make those two outcomes
indistinguishable at the call site.

`pty` chooses between two output shapes, and the choice is exclusive because a terminal is one
device. With `pty: true` the task is given a pseudo-terminal, a process asking whether it is
attached to a terminal is told yes, and **its output arrives merged on `execution/onStdout`** —
`onStderr` carries nothing, exactly as a real shell interleaves the two beyond separation. With
`pty: false` the task gets separate pipes, `onStdout` and `onStderr` are distinguishable, and the
process is not attached to a terminal. A terminal panel wants the first; a caller parsing a
build's errors wants the second, at the cost every CI system pays.

`cwd` and `env` are both optional, and their absent cases are the ones a caller most often wants.
An omitted `cwd` is the workspace root, which is where a build usually runs; an omitted `env` means
the task inherits the engine's environment unchanged, and a supplied one is merged **over** that
inheritance rather than replacing it. Replacement would be the more obvious reading of a bare
parameter and is the wrong default: a task started with a single variable set would lose `PATH`
and `HOME` and fail for a reason that looks nothing like its cause. The row formerly marked both
mandatory, which would have refused a caller who wanted exactly the defaults.

`command` is an **argv vector**, not a shell line. The engine does not interpose `sh -c`: §7.3
scopes this as process execution and not a shell, and a single string would make quoting the
engine's problem for input it is specifically required not to interpret. A caller that wants a
shell asks for one as `argv[0]`, which is a decision it has made rather than one made for it.

`data` on `writeStdin`, `onStdout` and `onStderr` is **base64**. A JSON string holds Unicode text
and a task's bytes are not text: a compiler emitting a byte sequence in the source file's own
encoding, a binary written to stdout, and a file catted into a terminal are all ordinary and none
of them survives a lossy decode, which substitutes U+FFFD and destroys the bytes it cannot read.
`workspace/readFile` reached the same conclusion and carries an explicit `encoding` field; these
payloads have no alternative encoding to select between, so it is fixed here instead of offered.

A `taskId` is **unique across the engine**, not within a workspace. Six of the nine rows address a
bare `taskId`, so a per-workspace identity would leave them unable to resolve a task at all. The
`workspaceId` on `runTask` and `attach` records which workspace owns the task, not which namespace
its name lives in — two workspaces both choosing `build` have named the same task, and the second
`runTask` is refused rather than silently starting a second process under a live identity.

`cols` and `rows` are optional on `runTask` and meaningful only when `pty` is true. A process
reads its terminal width at startup, before any client has had an opportunity to resize it, so
without them it reads whatever the pseudo-terminal happened to be created with rather than a value
somebody chose. Omitted, they default to **80 by 24** — the conventional terminal size, and
specifically not the kernel's own default of zero by zero, which is both a size no display has and
the one value `resizePty` refuses. `resizePty` against a task started with `pty: false` is **silently ignored**:
there is no terminal to resize, and a notification has no way to refuse.

`execution/onExit` carries `exitCode` **or** `signal`, exactly one of the two and never both. A
mandatory `exitCode` would leave a signalled death representable only through the `128 + n`
convention, which is what a shell does for a human reading a number, not what a protocol should
require a client to decode. An exit is two distinct states and the wire names which one occurred.

`execution/attach`'s result carries `exitCode?` and `signal?` under the same rule. A client that
reattaches to a task which finished while it was away learns how it finished from the response;
`running: false` on its own says only that it is over. `retained` is a **byte count**, not the
bytes themselves: the retention bound is larger than §4.1's frame cap, chunking is defined for
notifications rather than results, and an exit delivered inside the result would arrive before the
output that preceded it. The retained bytes are replayed after the response as ordinary
`onStdout` and `onStderr` notifications — **each chunk on the notification its own stream would
have used when live** — in order, so one ordering rule covers live and replayed output alike.
Replaying everything on `onStdout` would merge the two streams for a `pty: false` task, which is
the separation that task asked for by not requesting a terminal.

`signal` on `execution/terminate`, and on `onExit` and `attach` where they report one, is the
signal's **name** — `SIGINT`, `SIGTERM`, `SIGKILL` — not its number. Signal numbers differ between
platforms and the client is not always on the engine's; a client on macOS or Windows composing a
stop request should not have to know Linux's numbering, and an unrecognised name can be refused
whereas an unrecognised number is indistinguishable from a valid one. The engine is the only party
that needs the number, and it is the only party that has it natively.

The signal a client sends is the **initial** signal, and whether it escalates follows from which
one it is. `SIGTERM` escalates to `SIGKILL` after a grace period, because a stop that a process
can decline is not a stop. `SIGINT` does not escalate: it is the developer asking a foreground
process to stop the way Ctrl-C asks, and a program legitimately handling it — a test runner
printing a summary, a shell returning to its prompt — must not then be killed for having handled
it. A client that wants the process gone asks for `SIGTERM`.

Two limitations of this shape, stated rather than left to be discovered. A client cannot ask for a
`SIGTERM` that does **not** escalate, so a process that legitimately needs longer than the grace
period to shut down — a database flushing, a container stopping — is killed partway. No
requirement asks for "ask and wait", so no parameter exists for it; if one is added later it
belongs on `terminate` as a grace period, not as a second method. And `execution/list` is
**unpaged**, unlike `workspace/readDirectory`, which caps at a thousand entries and returns a
cursor. Its result is bounded only by how many tasks one developer has started, and a large enough
set would exceed §4.1's frame cap and answer `-32007` against the engine's own listing. That is
accepted because the realistic count is tens, and recorded because the arithmetic does not care.

`execution/list` exists because `attach` takes an identity the caller must already know. A client
that has lost its identities — a fresh install, a cleared profile, a crash before its store was
written — has no route back to tasks that are still running, and under A-TASKLIFE those tasks keep
running. Without enumeration they stay unreachable until A-EC2's idle stop ends the instance,
which is precisely the abandoned process FR-025 forbids, arrived at by a client doing nothing
wrong. `workspaceId` is optional: omitted, it lists every task the engine holds. Each entry carries
`taskId`, `workspaceId`, `command`, `pty`, `pid`, `running`, and `exitCode?`/`signal?` under the
same exactly-one rule as `onExit`. It deliberately does **not** carry `env`: FR-005a keeps a task's
environment out of anything that can be read back, and a listing is exactly that.

Attaching with a `workspaceId` that does not own the named task is **refused** with `-32001`, not
ignored. Because a `taskId` is engine-unique the engine could resolve the task from the id alone
and treat the mismatched workspace as noise, but a client that believes a task belongs to a
different workspace than it does is a client whose state has diverged, and silently servicing the
request would leave it diverged. Principle VI puts the check on both sides of the boundary.

### Git

| Method | Kind | Params | Result |
|---|---|---|---|
| `git/getStatus` | request | `workspaceId` | `{currentBranch, changes[]}` |
| `git/onStatusUpdate` | notification | `workspaceId`, `currentBranch`, `changes[]` | — |
| `git/getFileDiff` | request | `workspaceId`, `relativePath` | `{added[], deleted[], modified[]}` |

`changes[]` entries are `{path, status}` where status is `MODIFIED`, `UNTRACKED`, `STAGED`,
`DELETED` or `CONFLICT`. Diffs return line coordinates only, never file contents — the client
already holds the text and only needs gutter decorations.

---

# 5. Local Cache and Virtual File System

## 5.1 Purpose and authority

The client maintains a SQLite projection of the workspace so the UI can render without waiting
on the network. In remote mode this cache is **a projection, never an authority**. Where it
disagrees with the engine, the engine wins.

The cache serves three jobs: instant file tree rendering, avoiding refetches of unchanged
files, and read-only access when disconnected (§11).

## 5.2 Canonical schema (A-B5)

This schema is canonical. The original document carried two divergent versions; this is the
one the access patterns target, with the corrections the review required.

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE workspaces (
    workspace_id    TEXT PRIMARY KEY,   -- UUID
    name            TEXT NOT NULL,      -- display name
    location_type   TEXT NOT NULL,      -- 'REMOTE' | 'LOCAL'
    base_path       TEXT NOT NULL,      -- root on whichever machine owns it
    ssh_host        TEXT,               -- NULL when LOCAL
    last_opened_at  INTEGER NOT NULL
);

CREATE TABLE files (
    file_id            TEXT PRIMARY KEY, -- opaque UUID, NOT derived from path
    workspace_id       TEXT NOT NULL,
    parent_path        TEXT NOT NULL,    -- '/src/controllers'
    relative_path      TEXT NOT NULL,    -- '/src/controllers/user.go'
    name               TEXT NOT NULL,    -- 'user.go'
    is_directory       INTEGER NOT NULL,
    size_bytes         INTEGER NOT NULL,
    remote_modified_at INTEGER NOT NULL,
    last_cached_at     INTEGER,          -- when the blob was written
    last_accessed_at   INTEGER,          -- when the blob was last read; drives eviction
    is_cached          INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id) ON DELETE CASCADE,
    UNIQUE(workspace_id, relative_path)
);

CREATE INDEX idx_files_parent ON files(workspace_id, parent_path);
CREATE INDEX idx_files_lookup ON files(workspace_id, relative_path);

CREATE TABLE file_contents (
    file_id      TEXT PRIMARY KEY,
    content_blob BLOB,                   -- Zstd level 3
    sha256_hash  TEXT NOT NULL,          -- hash of the DECOMPRESSED content
    FOREIGN KEY(file_id) REFERENCES files(file_id) ON DELETE CASCADE
);

CREATE TABLE git_status (
    file_id       TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    status_type   TEXT NOT NULL,         -- MODIFIED|UNTRACKED|STAGED|DELETED|CONFLICT
    FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id) ON DELETE CASCADE
);

CREATE INDEX idx_git_status_lookup ON git_status(workspace_id, relative_path);

-- Fuzzy path search. A leading-wildcard LIKE cannot use an index and degrades to a
-- full scan on large workspaces, which is the opposite of instant filtering.
CREATE VIRTUAL TABLE files_fts USING fts5(
    relative_path,
    name,
    content='files',
    content_rowid='rowid'
);

-- An external-content FTS5 table stores only the index and reads column values back from
-- `files`. SQLite does NOT keep it in step: without these triggers the table is created
-- empty and stays empty, and every offline path search returns nothing -- quickly, and with
-- no error. The 'delete' command row must carry the OLD values, because the index cannot
-- read them back from a row that is already gone.
CREATE TRIGGER files_fts_insert AFTER INSERT ON files BEGIN
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;

CREATE TRIGGER files_fts_delete AFTER DELETE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
END;

CREATE TRIGGER files_fts_update AFTER UPDATE OF relative_path, name ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, name)
    VALUES ('delete', old.rowid, old.relative_path, old.name);
    INSERT INTO files_fts(rowid, relative_path, name)
    VALUES (new.rowid, new.relative_path, new.name);
END;
-- The `OF relative_path, name` clause arrived with schema version 2 (F004). Version 1 shipped
-- this trigger as `AFTER UPDATE ON files`, which fires a delete-and-reinsert for every update
-- including ones that change no indexed term — and F004 marks whole trees stale, which is
-- exactly that. Version 2's migration drops and recreates it; a version 1 database in the
-- field still carries the wider form until it migrates.
```

Four corrections are load-bearing:

- **`file_id` is an opaque UUID**, not a hash of the path. Path-derived identity meant a rename
  produced a new identity and orphaned the cached blob. Renames now update `relative_path`,
  `parent_path` and `name` in place, and the cached content survives.
- **`last_accessed_at` exists.** The eviction policy (§5.5) evicts blobs unopened for a period,
  which the original schema could not express because it stored only write time.
- **`files_fts` exists.** Offline path search was specified as a leading-wildcard `LIKE`, which
  cannot use an index.
- **`files_fts` has synchronisation triggers.** The table is declared `content='files'`, which makes
  it *external-content*: it holds the index and reads the values back from `files`. SQLite does not
  maintain such a table on its own, and the original schema declared the table with no triggers — so
  as written the index was created empty and stayed empty, and offline path search returned no rows
  with no error. Triggers rather than adapter code because they make correctness structural: no
  write path can forget to update the index, since no write path is involved, and F004's watcher and
  F012's offline writes are exactly the future writes that would forget.

## 5.3 Cache validity

A cached blob is valid when `file_contents.sha256_hash` equals the engine's current hash for
that path. Nothing else invalidates it.

In particular, **git status does not invalidate the cache**. A file the user has just edited and
saved is `MODIFIED` in git; invalidating on that signal would discard cached content for exactly
the files being worked on, forcing a refetch on every save and making those files unreadable
offline — the one situation the cache exists for. Git status colours the tree; it is not a cache
signal.

## 5.4 Access patterns

**Sidebar render.** Query by parent path; no network round trip.

```sql
SELECT name, is_directory, is_cached, size_bytes
FROM files
WHERE workspace_id = ? AND parent_path = ?
ORDER BY is_directory DESC, name ASC;
```

**Open a file.** Read the cached hash and content together, then confirm against the engine.
A hash match serves from cache; a mismatch or a miss issues `workspace/readFile`.

```sql
SELECT f.file_id, f.is_cached, c.sha256_hash, c.content_blob
FROM files f LEFT JOIN file_contents c ON f.file_id = c.file_id
WHERE f.workspace_id = ? AND f.relative_path = ?;
```

Every cache hit updates `last_accessed_at`.

**Path search offline.** Query `files_fts`, not `LIKE`. Online, path and content search both go
to the engine, which runs ripgrep across the real tree (§10.2).

## 5.5 Eviction

Content blobs unopened for **14 days** are deleted; file tree metadata is retained. Eviction
never removes rows from `files`, only from `file_contents`, and clears `is_cached`. This keeps
the tree navigable while bounding disk use.

## 5.6 Compression

Content blobs are Zstd level 3. Level 3 balances ratio against decompression speed, and
decompression is fast enough to stay off the interaction path. `sha256_hash` is computed over
the decompressed content so it can be compared directly with the engine's hash.

---

# 6. Engine Abstraction

## 6.1 The provider trait

Both modes implement one trait. The UI never learns which is active.

```rust
#[async_trait]
pub trait WorkspaceProvider: Send + Sync {
    async fn read_directory(&self, path: &RelPath) -> Result<Vec<FsEntry>>;
    async fn stat(&self, path: &RelPath) -> Result<FsMeta>;

    /// Ranged read. `range: None` reads the whole file.
    /// Returns bytes, not String: build artifacts, images and PDFs are legal content.
    async fn read_file(&self, path: &RelPath, range: Option<ByteRange>) -> Result<FileChunk>;

    /// Rejects when `base_sha256` no longer matches, rather than overwriting.
    async fn write_file(&self, path: &RelPath, content: &[u8], base_sha256: &Sha256)
        -> Result<Sha256>;

    async fn create_file(&self, path: &RelPath) -> Result<Sha256>;
    async fn create_directory(&self, path: &RelPath) -> Result<()>;
    async fn rename(&self, from: &RelPath, to: &RelPath) -> Result<()>;
    async fn delete(&self, path: &RelPath, recursive: bool) -> Result<()>;

    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchMatch>>;

    async fn watch(&self, paths: &[RelPath]) -> Result<WatchOutcome>;
    async fn unwatch(&self, paths: &[RelPath]) -> Result<WatchOutcome>;
}
```

`watch` took one path and returned a `WatchHandle` until F004. Three things were wrong with that.
A handle implies a subscription the caller later drops, which is the wrong lifetime model once
watching is a set the engine reconciles — and no `WatchHandle` type was ever defined anywhere, so
the normative signature referred to nothing. One path at a time cannot re-establish a whole set on
reconnection without the client replaying a remembered history. And there was no way to stop
watching at all. Taking a slice and returning the outcome makes the trait say what A-WATCHSCOPE
decided; `WatchOutcome` carries the count now watched and the paths refused.

The signature is normative in three respects the original draft got wrong. `read_file` returns
bytes rather than `String`, because the system is required to serve binary build artifacts.
It takes a range, because large files are streamed as the user scrolls. And the trait includes
create, rename, delete and watch, without which an IDE cannot function.

## 6.2 Path resolution

`RelPath` is workspace-relative and is validated on construction. Providers resolve it against
their base path and assert containment (§4.7). A provider MUST NOT join untrusted input to a
base path without that check.

## 6.3 Remote provider

Translates each call into the corresponding JSON-RPC method and dispatches through the
correlation registry (§4.3). It resolves when the reply arrives — never on send.

## 6.4 Local provider

Maps directly onto `tokio::fs` and native syscalls. Used by Local Mode (§13) and subject to the
same containment rule: a local workspace is still a workspace, and path escapes are still bugs.

---

# 7. Language Intelligence

## 7.1 Supported toolchains

| Language | Toolchain | Language server |
|---|---|---|
| JVM (Java) | `javac` / `java` | Eclipse JDT LS |
| Python | `python3` / `pip3` | Pyright, with Ruff for linting |
| Go | `go` | gopls |
| Elixir | `elixir` / `mix` | Lexical |
| Rust | `cargo` / `rustc` | rust-analyzer |
| Zig | `zig` | zls |
| C / C++ | `gcc` or `clang` | clangd |

## 7.2 Multiplexing

The engine runs native language servers as child processes and routes LSP traffic to them by
`language` and `workspaceId`. LSP payloads pass through unmodified (§4.8); the engine is an
envelope and a router. Nothing reimplements language analysis.

This is the design's central leverage: rust-analyzer indexing a large dependency graph uses the
instance's memory, not the laptop's.

## 7.3 Server lifecycle

The supervisor handles four transitions per server:

- **Spawn** — on first need for a language in a workspace, with the right working directory and
  environment. Not at connection time; a Go-only workspace never starts jdtls.
- **Monitor** — health and liveness.
- **Remediate** — restart crashed servers with backoff, and emit `lsp/onServerState` so the UI
  distinguishes "no results" from "server died".
- **Terminate** — clean `SIGTERM` on workspace close and client disconnect, so a dropped
  connection does not leave orphaned servers holding memory.

Child processes run under Linux cgroups so a runaway indexing task cannot destabilise the
orchestrator. **Execution tasks are not yet among them**: F010 bounds them with per-process
limits and a process group instead, because cgroup delegation depends on provisioning F005 has
not specified. See A-TASKLIMIT, which records what that does and does not catch.

One server per language per workspace, started lazily on first use of that language, each under
a cgroup memory limit. A server exceeding its limit is killed and restarted rather than allowed
to exhaust the instance — under §15.5's single-tenant model, one runaway server would otherwise
take down the developer's whole environment. One workspace is one root; multi-root workspaces
are not supported. Client capabilities pass through to the server unmodified. See Appendix A,
A-LSP.

## 7.4 Debugging

The engine acts as a DAP broker, fronting `delve` (Go), `lldb-vscode` (Rust, C, C++, Zig) and
`debugpy` (Python).

**Out of scope for v1.** Debugging is not specified, not built, and reserved for in neither §4
nor the build sequence in §19. The section is kept because the intent above is the shape any
future support would take, not because anything implements it. Reopening it is a product
decision that adds at least one feature to the map. See Appendix A, A-DAP.

---

# 8. Editor and Interface

## 8.1 Monaco

The editor is Monaco, rendered in the webview. Writing a text rendering engine — typography,
selection, multi-cursor, line wrapping, international text — is years of work with no product
differentiation.

Monaco's decoupling from file systems and analysers is what makes it fit here. Two integration
points matter:

**The text model is local.** Opening a file populates a `TextModel` from cache or from the
engine. Typing mutates it immediately, with no network in the path. This is rule 1 of §1.5.

**The intelligence providers are remote.** Monaco's local web workers are disabled. Completion,
hover, definition, references and diagnostics are registered against providers that translate
into `lsp/request` and return the engine's response. Syntax highlighting stays local via
Tree-sitter, so colour survives a dropped connection.

## 8.2 Large files

Monaco virtualises rendering, creating DOM elements only for visible lines, so file size does
not drive render cost. The client complements this with ranged reads (§4.6) rather than
fetching a whole large file before first paint.

## 8.3 Terminals

Xterm.js, one instance per task or shell, fed by `execution/onStdout` and `onStderr`. Input
flows back through `execution/writeStdin`, and resize through `execution/resizePty`, so remote
processes behave like a real terminal rather than a log viewer.

## 8.4 Status and mode indication

The status bar always shows connection state. Offline is unmistakable (§11.1) rather than
inferred from things silently failing.

---

# 9. Execution and Build Artifacts

Code executes on whichever machine owns the workspace. Artifacts are classified by how they
reach the user.

## 9.1 Text output

Process stdout and stderr stream as notifications and render in Xterm.js with full ANSI
handling. This covers build logs, test output and run consoles.

## 9.2 File artifacts

The engine watches build output directories. When a watched file appears or changes, it emits
`workspace/onFileEvent`. If the client has that file open in a viewer, it fetches the bytes —
over SFTP when large (§3.6) — and renders them. Images, PDFs and generated media are ordinary
files; §6.1 returns bytes precisely so this works.

## 9.3 Network previews

When user code serves HTTP, the client opens a dynamic local forward (§3.5) and renders
`http://127.0.0.1:<local>` in a preview pane.

Previews are **user-initiated**: the developer names the port and starts the preview. Nothing is
forwarded without being asked for, which is what keeps the security story trivial — a port is
forwarded because somebody asked for that port, not because a heuristic guessed. Auto-detection
remains addable later, since an explicit action is a subset of an offered one. See Appendix A,
A-PREVIEW.

## 9.4 Native GUI output

Out of scope for v1. Should it be revisited, the shape is a headless framebuffer on the engine
(Xvfb or TurboVNC) with an encoded pixel stream tunnelled over the existing connection. Nothing
in the current design depends on it.

---

# 10. Large Codebases

The client never indexes. Every technique here follows from that.

## 10.1 Lazy tree loading

Opening a workspace fetches only the root listing. Expanding a folder consults SQLite; on a
miss it issues one `workspace/readDirectory` for that folder's immediate children and caches
the result. Tree cost is therefore proportional to what the user has actually opened, not to
repository size.

## 10.2 Remote indexing and search

Language servers index on the engine, holding their structures in its memory. Global search
runs there too — `ripgrep` across the real tree, or the language server's index for symbol
queries — and returns a bounded result list. The client renders text it did not compute.

## 10.3 File watching

The engine owns all file watches, using `inotify` scoped to **what the client has asked to
watch** — expanded folders, the directories holding open files, the ancestors of both, and the
workspace root — with build and dependency directories excluded. Watch cost is therefore
proportional to what the developer has opened rather than to the size of the repository, which is
the same principle §10.1 applies to the tree itself. The client says what it cares about through
`workspace/watch` and `workspace/unwatch` (§4.8); see A-WATCHSCOPE for why the engine cannot infer
it. Where the host's watch capacity is exhausted even under that scope, the engine reports the
refusal rather than appearing to watch, and the workspace remains browsable. Where the kernel's
own event queue overflows, the engine emits `workspace/invalidateAll` (§10.4), because events the
kernel dropped are changes nobody would otherwise hear about.

The exclusion set is the repository's own `.gitignore` files plus a fixed built-in set —
`.git/`, `node_modules/`, `target/`, `dist/`, `build/`, `.venv/`, `__pycache__/`. One resolved
set is computed per workspace and used by **both** the indexer and the watcher, so disagreement
is impossible by construction: an indexer that indexes what the watcher ignores returns search
results for files whose changes are never noticed. No per-workspace user configuration in v1.
See Appendix A, A-IGNORE.

The client sets no OS watches in remote mode. It receives `workspace/onFileEvent` and acts only
on paths it is currently displaying.

## 10.4 Bulk invalidation

Operations that change many files at once — branch switches, large pulls — emit
`workspace/invalidateAll` rather than thousands of individual events. The client marks its tree
stale and re-queries lazily as the user navigates. Cached content blobs remain valid or not on
their own hash terms (§5.3); invalidation of the tree is not invalidation of content.

---

# 11. Offline Behaviour and Reconnection

## 11.1 Decision (A-B3): read-only offline

When the connection to the engine is lost, the workspace becomes a **read-only mirror**. The
editor locks, the cached tree stays navigable, cached files stay readable, and nothing is
queued for later write.

**This decision is provisional and was made by engineering, not product.** It resolves a direct
contradiction in the source material, which specified both an outbound write queue and a
read-only editor lock. The alternatives, and what reversing this costs, are in Appendix A, A-B3.

The consequence worth stating plainly: a developer on a plane can read their code and cannot
change it.

## 11.2 What the user sees

Offline is explicit, never inferred from silent failure:

- The status bar shows a distinct offline state.
- Open tabs stay open; the tree stays interactive; nothing closes or resets.
- Monaco models switch to `readOnly: true`.
- Features that require the engine state that they require the engine, rather than appearing
  broken.

## 11.3 Component behaviour

| Component | Online | Offline |
|---|---|---|
| File tree | SQLite, lazily filled from the engine | SQLite only; unfetched folders marked unavailable |
| Editor | Cache or ranged read from engine | Cached files only, read-only |
| Path search | Engine-side ripgrep | `files_fts` over cached paths |
| Content search | Engine-side ripgrep | Unavailable, stated as such |
| Code intelligence | Remote language servers | Unavailable; syntax highlighting persists (local Tree-sitter) |
| Terminals and tasks | Streaming from engine | Unavailable; existing output stays readable |
| Git status | Engine-computed, cached | Last known state, marked stale |

## 11.4 Prefetch

To make offline useful rather than merely non-destructive, the client caches deliberately while
online: every file opened, plus a background prefetch of files changed in recent commits and
project manifests (`go.mod`, `Cargo.toml`, `package.json`, `mix.exs`, `pyproject.toml`).

Prefetch is background-priority (§4.6) and must never delay interactive traffic.

## 11.5 Reconnection

A tokio loop attempts reconnection on a backoff while offline. On success:

1. Re-establish the connection per §3.1 and §3.3.
2. Re-run the handshake and verify protocol compatibility (§3.8).
3. Reconcile: compare cached hashes against the engine for open files; refetch what changed.
4. Restore language servers for open workspaces.
5. Unlock the editor and update the status bar.

Reconciliation is a **three-way merge**, not a pull: edits made offline persist against the
`baseSha256` the client held when the connection dropped, and on reconnect the client merges
base, local and remote per file. Where the remote has not moved it fast-forwards; where it has,
only genuinely colliding hunks raise a conflict for the developer to resolve.

A merge that silently picks a side is a merge nobody can audit, so conflicts prompt rather than
resolve themselves. See Appendix A, A-OFFLINE.

Conflict resolution is in scope. A-OFFLINE reversed the read-only call that had made it moot, so
this section requires a stored base revision in `file_contents` (§5.2), a three-way merge, and a
conflict interface. The base revision is not a new protocol concept: `workspace/writeFile`
already carries `baseSha256` so the engine can refuse a stale write with `-32004`, and offline
editing reuses exactly that value.

---

# 12. Git Integration

## 12.1 Division of work

Git computation runs on the engine. The client stores a minimal projection sufficient to colour
the tree and draw gutters. `git status` across a large repository on a laptop is exactly the
kind of work this architecture exists to move.

## 12.2 Status pipeline

The engine watches `.git/HEAD` and `.git/index`. On change it runs
`git status --porcelain=v2 -z`, parses it, and emits `git/onStatusUpdate` carrying the current
branch and a list of changed paths.

The client applies the update in one transaction: clear the workspace's `git_status` rows,
insert the new set, done.

It does **not** touch `is_cached`. See §5.3 — the file the user just saved is `MODIFIED`, and
invalidating on that signal destroys the cache for exactly the files in active use.

## 12.3 Presentation

The tree joins `files` against `git_status` to colour entries by state. The status bar shows the
current branch. Opening a modified file issues `git/getFileDiff`, which returns line coordinates
only; the client feeds them to Monaco's decoration API to draw the gutter. File contents are
never transmitted for diff purposes — the client already has the text.

## 12.4 Branch switches

A branch switch can change an enormous number of files. The engine updates its own indexes and
emits a single `workspace/invalidateAll` (§10.4). The client does not download the new branch
and does not rebuild its tree eagerly.

---

# 13. Local Mode

## 13.1 Purpose

A workspace may target the developer's own machine instead of an instance. The provider
abstraction (§6) makes this a routing decision rather than a second application.

Local mode is not the same as offline (§11). An offline workspace is a remote workspace whose
engine is unreachable, and is read-only. A local workspace has no remote at all and is fully
writable.

## 13.2 Behaviour

File operations use native syscalls. Language servers are spawned from the user's own
installation. Tasks run in a local PTY via `portable-pty` against the user's shell — **built by F015
`local-mode`, not by F010**, whose scope is the remote engine. Until F015 lands, the client's
local provider refuses task methods and says so. The SQLite
cache still serves tree rendering and search, as a performance layer rather than a network
mask.

## 13.3 Mode comparison

| | Remote | Local |
|---|---|---|
| Compute | 16 vCPU / 128 GB instance | The laptop |
| Network | Required | None |
| Source of truth | Engine filesystem | Local filesystem |
| Cache role | Masks network latency | Speeds tree and search |
| Toolchain prerequisite | None on the laptop | Compilers and LSPs installed locally |
| Offline editing | No (§11.1) | Not applicable — always local |

---

# 14. Toolchain Detection and Remediation

## 14.1 Detection

On opening a workspace, or first encountering a file type, the client checks asynchronously for
both the toolchain and its language server, per §7.1. Detection resolves binaries on `PATH` and
falls back to common version managers (`asdf`, `mise`, `nvm`, `pyenv`). It never blocks the UI.

In local mode this inspects the laptop. In remote mode the engine inspects itself and reports.

## 14.2 Presentation

Findings surface as an editor banner above the document, not a modal:

- **Toolchain present, language server missing** — offer to install the server.
- **Neither present** — offer to install the toolchain, or to move the workspace to an instance
  where it is already configured (§15.4).

## 14.3 Remediation constraints

Automated installation runs **pinned and verified, and fails closed**. Every toolchain is pinned
to an exact version and checked against a hash recorded in the client before execution;
verification failure aborts and reports rather than falling back to an unverified copy, because
a fallback path is the one that runs precisely when verification failed. Nothing escalates
privilege: under §15.5 the instance is the developer's own, and a toolchain needing root on a
single-user machine is being installed in the wrong place. This is the same integrity mechanism
as §3.8's deployment check, deliberately — one way of saying "this binary is the one we meant",
not two. See Appendix A, A-D12.

Concretely this requires: pinned versions in the dependency manifest, verification of
fetched artifacts, the exact command shown to the user before execution, and a defined failure
path. See Appendix B.

---

# 15. Remote Infrastructure

## 15.1 The orchestrator

`ide-engine` is a Rust binary using tokio, running on the instance as an ordinary user. It is
the sole entry point: LSP multiplexer, DAP broker, process supervisor, file watcher and
filesystem server. It speaks the protocol in §4 over stdio and nothing else.

It runs language servers under cgroups (§7.3) so no single server can destabilise it. Execution
tasks are bounded per process instead, by resource limits and a process group (A-TASKLIMIT); the
blanket claim that every child runs in a cgroup was true of the design §7.3 described before
cgroup delegation was found to depend on provisioning F005 owns.

## 15.2 Availability

Against the 99.9% target (§1.4), the engine tracks active task IDs, PID mappings and language
server session state in memory, so a client that **disconnects** and returns reattaches to work
that kept running rather than rebuilding its session by hand (A-TASKLIFE).

This does not extend to a crash. The map is memory and dies with the process, so a crashed engine
loses every identity it held while the child processes it started keep running — reachable by pid
and by nothing the protocol exposes. Recovering that needs the map to outlive the process, which
nothing in the system does today; until it does, the honest statement is that a disconnection is
survivable and a crash is not.

## 15.3 Updates

The engine supports in-place binary replacement and re-execution so toolchain updates do not
require the developer to intervene. This makes client/engine version skew a routine condition
rather than an exception, which is why §3.8 is blocking rather than cosmetic.

Running tasks are **terminated before the re-execution** and reported in `session/onRestart`'s
`unpreserved` list. See A-TASKEXEC.

## 15.4 Cloud burst

Moving a local workspace to an instance:

1. Verify the instance is reachable and the engine responsive.
2. Stream the project over SFTP (§3.6), excluding VCS internals, build output and local caches.
3. Register the workspace with the engine.
4. Switch the provider from local to remote and re-point path mapping in SQLite.
5. Warm the language servers for the detected toolchains.

A remote workspace is identified by an opaque `workspaceId` minted by the client, so it is
addressable before the engine has ever seen it. Its display name is the repository directory
name and **may collide freely**, because nothing keys on it — two checkouts of one repository
are the ordinary case, not an edge one. A second burst of the same workspace attaches to what is
already there rather than overwriting or forking. Deletion is explicit and removes both the
remote directory and the local cache. No quota: under §15.5 the instance is single-tenant, so a
developer filling their own disk is a problem they can see, and exhaustion surfaces as an
ordinary engine error. See Appendix A, A-WORKSPACE.

## 15.5 Instance lifecycle

Instances are **per developer**, never shared. An instance stops after 30 minutes without
interactive traffic — background indexing must not hold one awake, or nothing ever stops — and
the wake target is under 60 seconds from the developer's action to a usable editor. Beyond about
a minute developers start leaving instances running to avoid the wait, which defeats the policy
entirely. Address resolution is dynamic on wake, and security groups remain restricted to port
22.

Single tenancy is load-bearing rather than incidental: it is the condition under which §16.3's
authorization model is acceptable at all. See Appendix A, A-EC2.

---

# 16. Security Model

## 16.1 Trust boundaries

Three boundaries matter:

1. **Client to engine.** The engine accepts frames from the client and must not trust them.
   Path containment (§4.7) is enforced engine-side regardless of client-side validation.
2. **Engine to user code.** Builds and tests run arbitrary code from the repository, as the
   developer's own user. This is expected — it is what a build is. The bound differs by what is
   running. Language servers run under cgroups (§7.3). **Execution tasks do not**: they are bounded
   per process by an address-space limit, disabled core dumps and a process group, which stops one
   runaway and does **not** stop a process tree exhausting the instance collectively (A-TASKLIMIT).
   The remaining bound is that the instance is developer-owned and not shared (A-EC2). This
   paragraph previously claimed cgroups bounded everything, which stopped being true when §7.3 was
   amended and is corrected here rather than left as a security control nothing implements.
3. **Client to remote content.** The client renders remote-sourced content, including HTML
   previews of remotely served applications.

## 16.2 Controls

- **Port 22 only.** Security groups expose nothing else. Previews reach the developer through
  forwards inside the existing connection, never through opened ports.
- **Loopback binding.** Preview listeners bind `127.0.0.1` (§3.5). Binding `0.0.0.0` would
  expose the remote service to the developer's whole network.
- **Path containment.** Enforced on both sides (§4.7), rejecting traversal and escaping
  symlinks with `-32002`.
- **Host key verification.** Delegated to OpenSSH; changed keys abort the connection and are
  never auto-accepted (§3.9).
- **Webview sandbox stays enabled.** The original draft disabled it as a performance tweak,
  unmeasured, in an application that renders remote content. It remains on.
- **Secrets.** Passphrases are held in memory only, zeroed after use, never logged. A
  passphrase the user elects to remember goes to the OS keychain, not to application
  preferences.

## 16.3 Gaps

Authorization **is** the developer's SSH access to their own instance. There is no second
authorization layer, no per-client resource limit and no audit log, because under §15.5 that
client is the machine's only user. Building a token layer over a single-user machine would imply
a boundary that does not exist, and the honest model is the one stated plainly.

This decision is downstream of §15.5 and falls with it: any move to shared instances reopens
authorization, per-client limits and audit logging together. See Appendix A, A-SEC.

---

# 17. Client Packaging

## 17.1 Framework (A-B7)

**Tauri v2.** Tauri v1 is end of life; targeting it for a new build would mean migrating
before shipping. The original draft's manifest was a v1 schema (`tauri.allowlist`,
`package.productName`) while its prose claimed v2 — v2 replaced the allowlist with capabilities
and permissions, so that manifest would not have validated.

The permission set is scoped to what the app uses. Nothing is enabled speculatively.

## 17.2 Why a native webview

The client uses the OS webview — WebKit on macOS, WebKitGTK on Linux — rather than bundling a
browser engine. This is the main reason the client stays small while Monaco and Xterm.js
handle rendering, and it composites through the platform's own GPU path.

On Linux, WebKitGTK may fall back to software rasterisation depending on the display server.
Compositing is enabled explicitly. The sandbox is **not** disabled (§16.2).

## 17.3 Release build

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

`panic = "abort"` removes unwinding. The engine is supervised externally and the client restarts
cleanly, so unwinding buys nothing here.

## 17.4 Distribution

Targets are `.dmg` for macOS and `.deb` for Linux.

The macOS client ships **unsigned and un-notarized**, with the Gatekeeper bypass documented on
the download page. Subprocess spawning (§3.1) is permitted under the hardened runtime that
notarization would require, so signing remains available later without reopening the transport
decision; it would not survive App Store sandboxing either way.

The cost is recorded rather than glossed: every first launch is a security warning the user is
told to dismiss, which is the habit §3.9 refuses to train for host keys. The two are traded
against different costs, and the trade is written down in Appendix A, A-SIGN.

The client is delivered through platform package managers — an apt repository for Debian and
Ubuntu, a Homebrew cask for macOS — with no in-application updater. The package manager owns
integrity checking, so no separate update feed needs signing, which matters given §17 ships
unsigned.

Skew has exactly one remedy in exactly one direction: §3.8 makes the client the authority that
deploys the engine, so the client is always the half that needs updating, and the package
manager is how it gets updated. See Appendix A, A-UPDATE.

---

# 18. Verification and Operations

## 18.1 Network resilience testing

A mock SSH daemon simulates realistic conditions — 250 ms RTT, 5% packet loss — in CI. This
exists so the latency and resilience behaviour can be verified on every change without AWS
spend, and it is a deliverable of the first build increment, not an afterthought.

## 18.2 Test strategy

Every increment is accepted against four levels: unit coverage for domain and application logic,
integration coverage against the mock daemon, end-to-end coverage on Linux per Appendix A A-E2E,
and an opt-in suite against a real `sshd` for what no mock can prove — the §3.1 option set being
the canonical example, since a mock has no socket and no keepalive. A performance gate fails when
the interaction budget regresses, measured per §1.4.

One rule beyond coverage: a test that cannot fail is worse than no test, because it reports
confidence it has not earned. Where a check guards a property that would otherwise be invisible,
verify the check fails when the property is broken. See Appendix A, A-TEST.

## 18.3 Observability

Logs are structured, stay on the developer's machine with bounded retention, and are never
transmitted. There are no metrics and no usage telemetry, so no repository path or file name
leaves the machine. The single exception is a crash reporter that uploads a stack trace on an
unhandled panic and nothing else.

That exception carries a hard requirement: a panic payload is exactly where a secret is most
likely to surface, which §16 already defends against locally. Any crash reporter must redact
before transmission and be tested against a sentinel, or the existing assertion that a
passphrase reaches no panic payload becomes a lie. See Appendix A, A-OBS.

---

# 19. Build Sequence

Ordered by dependency, not by visibility. The backlog in `specs/features-map.md` carries these
as F000-F017. File position in that map is authoritative for build order; from the first
recorded spec onward, identities there are immutable.

## 19.1 Foundations

**F000 `app-shell`** — Tauri v2 scaffold (§17.1), window and dockable panel layout, the
webview-to-core IPC bridge, status bar (§8.4) and theming. Built first: it needs no network,
it is the integration point every later feature mounts into, and every feature owning UI
surface depends on it. Until F001 exists it renders static or stubbed state.

**F001 `ssh-transport-core`** — OpenSSH subprocess invocation (§3.1), the two-phase connect
sequence (§3.3), `Content-Length` framing, the correlation registry (§4.3), failure
classification (§3.4), and the mock SSH daemon (§18.1). Nothing *remote* precedes it, and it
carries the design's riskiest unknowns, so it should not trail F000 by long. F000 and F001
have no dependency between them and may proceed concurrently.

**F002 `daemon-bootstrap`** — deployment, handshake, version negotiation (§3.8). Unblocked
2026-09-23 by A-BOOT; the client pushes the engine over the connection it already holds and is
the authority on protocol version.

**F003 `workspace-cache`** — the `WorkspaceProvider` trait (§6.1), canonical schema (§5.2),
lazy tree loading (§10.1), ranged reads, hash-based validity (§5.3). First point at which a
real repository can be browsed.

**F004 `file-watch-sync`** — engine-side watching (§10.3), `workspace/onFileEvent` emission,
the ignore set, and client-side invalidation (§10.4). Without it the client never learns that
a file changed underneath it.

**F005 `ec2-lifecycle`** — wake, stop, address resolution (§15.5). Unblocked 2026-09-23 by
A-EC2: per developer, 30-minute idle stop, under 60 seconds to wake.

## 19.2 First usable product

**F006 `editor-integration`** — Monaco with local text models (§8.1) and writes carrying
`baseSha256` (§4.8). First point at which the product does its job.

## 19.3 Language support

Split because seven integrations cannot be one honest feature, and because the first one
proves a pattern the rest repeat.

**F007 `lsp-multiplexing`** (§7.2, §7.3) — the language-agnostic mechanism: spawn,
supervision, cgroup isolation, envelope routing, Monaco providers, cancellation.

**F008 `language-toolchains`** (§7.1, §14.1) — what the mechanism assumes exists: the language
manifest, per-language workspace root and configuration contract, Tree-sitter grammar bundling
for client-side highlighting, engine image toolchain provisioning, and Go end to end as the
reference integration.

**F009 `language-pack`** (§7.1) — the remaining six, each a repetition of the proven pattern:
Python, Rust, JVM, C/C++, Zig, Elixir.

## 19.4 Capability fan-out

Independent of one another once their dependencies land:

**F010 `execution-terminals`** (§4.8, §8.3) · **F011 `git-integration`** (§12) ·
**F012 `offline-readonly`** (§11) · **F013 `global-search`** (§10.2, §5.4) ·
**F015 `local-mode`** (§6.4, §13, §14)

F012 is deliberately after the online path is solid — offline behaviour is defined relative to
online behaviour.

## 19.5 Composite and delivery

**F014 `client-packaging`** (§17.4) — unblocked 2026-09-23 by A-SIGN and A-UPDATE, and nothing
ships without it. Note its scope grew with those decisions: package-manager delivery is three
pipelines rather than one installer. **F016 `cloud-burst`** (§15.4) — requires both modes working.
**F017 `previews-artifacts`** (§3.5, §9.2, §9.3) — requires execution.

## 19.6 Not scheduled

**Debugging** (§7.4) is out of scope for v1 by decision, not by omission (A-DAP). Reopening it
is a product call that adds at least one feature here, and it would also need a per-language
pass equivalent to F009: delve, lldb-vscode and debugpy.

**Observability** (§18.3) is settled at local logs plus a redacting crash reporter (A-OBS),
which needs no feature of its own: logging already exists, and the crash reporter is a bounded
addition to F014's packaging work rather than a separate increment.

Both are absent from the map by intent, not oversight. Neither blocks anything on it.

---

# Appendix A — Decision Record

Binding. Dated, and superseded rather than edited in place.

## A-B1 — SSH transport: spawn the OpenSSH client (2026-09-20)

**Decision.** The client spawns the system `ssh` binary and speaks length-prefixed JSON-RPC over
its stdio. No SSH library is linked in. Single stdio pipe for control traffic.

**Rationale.** Authentication ownership was the deciding axis. OpenSSH already implements the
agent protocol, `~/.ssh/config`, `known_hosts` with hashed hosts and revocation, certificates,
`ProxyJump`, FIDO `sk-` keys and PKCS#11 tokens — correctly, and on machines developers already
have. Reimplementing that surface is security-critical work with no product differentiation, and
any gap in it locks out a class of enterprise user. The decision deletes a planned feature
rather than adding one: authentication collapses to process invocation plus error
classification.

**Rejected — russh.** Tokio-native, typed errors, native prompt routing, programmatic channels.
Transfers the whole authentication surface to us, and lacks `sk-`/PKCS#11 and OpenSSH
certificates, so a developer with a hardware token could not connect.

**Rejected — `ssh2`/libssh2.** What the original draft's sample code used. Pays the costs of
both alternatives: a blocking C API needing a dedicated-thread actor to stay off the tokio
workers, *and* no `ControlMaster`, *and* no `~/.ssh/config`, *and* no certificate, `ProxyJump` or
FIDO support. Its unique advantage is a FIPS-validated crypto path via OpenSSL. **If FIPS
becomes a requirement this decision must be revisited** — it is the only option that satisfies
it.

**Known costs accepted.** Error detection depends on classifying OpenSSH's stderr rather than
typed errors, mitigated by `LC_ALL=C` and a bounded failure set (§3.4). GUI passphrase prompts
require an askpass helper (§3.3). Behaviour depends on the user's OpenSSH build, mitigated by
preflight (§3.10).

## A-B2 — Preview forwarding permitted (2026-09-20)

**Decision.** Local forwards are used for HTTP previews only, added to the live master
connection, bound to loopback, allocated per preview window.

**Rationale.** The claim that forwarding is inherently insecure was wrong, and contradicted the
document's own preview requirements. A `-L` forward and a library's `direct-tcpip` channel are
the same wire mechanism inside the same encrypted connection. What matters is the listener's
bind address — which the original fixed-port design got wrong by implying static, always-open
tunnels. Because A-B1 provides a `ControlMaster`, `-O forward` mutates the authenticated
connection with no new handshake.

## A-B3 — Read-only offline (2026-09-21, provisional) — SUPERSEDED

**Superseded by A-OFFLINE (2026-09-23).** Product answered the question this entry left to
them and reversed it: offline editing is a differentiator worth its cost. The entry is kept
in full rather than edited, because the reasoning below is still the argument against, and a
decision record that quietly becomes its own opposite teaches nobody anything.

**Decision.** Losing the connection makes the workspace a read-only mirror. No write queue.

**Status: provisional, engineering-made.** The source material specified both an outbound write
queue and a read-only editor lock, which cannot both be true. This resolves the contradiction
in the direction that minimises scope. Product may reverse it.

**Rationale.** It requires no outbox table, no stored base revision, no merge algorithm and no
conflict UI. It matches how comparable remote-development tools behave on disconnect. It makes
A-B4 unnecessary. And it cannot lose work, because it never accepts work it cannot commit.

**Rejected for now — outbound queue.** Genuine differentiation, and the honest cost is roughly
two additional features: persistence plus reconciliation. It forces A-B4 to be solved properly,
requiring a base revision column, three-way merge, and a conflict UI for when CI or a pull moved
the file. Reversing A-B3 means rewriting §11 and reopening B4; nothing else in this document
depends on it.

**Rejected for now — dirty buffers with explicit reconcile.** The middle option: keystrokes
persist locally so work is never lost, but nothing auto-writes; on reconnect a hash comparison
either fast-forwards or opens a review. Cheaper than a full outbox because detection needs only
the hash already stored. **This is the natural second step** if read-only proves too
restrictive, and is preferred over the full queue.

## A-B5 — Canonical cache schema (2026-09-21)

**Decision.** One schema, §5.2, replacing two divergent versions.

**Rationale.** The variant labelled "formal" in the source omitted the `name` column its own
sidebar query selected, and omitted the `git_status` table defined elsewhere in the same
document. The other version is the one the access patterns target, so it is the base.

Three corrections were made rather than inherited: opaque `file_id` so renames preserve cached
content; `last_accessed_at` so the eviction policy is expressible at all; an FTS table so
offline path search does not degrade to a full scan.

## A-B6 — Canonical protocol contract (2026-09-21)

**Decision.** One contract, §4, defined as the union of the source document's two versions plus
the methods required to make the described features work.

**Rationale.** The section labelled the formal contract defined five methods and omitted
`lsp/request`, `execution/onStdout`, `git/onStatusUpdate`, `git/getFileDiff` and
`workspace/invalidateAll`, all of which the body treats as load-bearing. It also dropped the
workspace identifier from `readDirectory`, which silently removed multi-workspace addressing.

Added because their absence made specified features impossible: an error model, cancellation
(required by the debounce rule), task stdin, resize and termination (the terminal was otherwise
write-only with no way to stop a process), a file-watch method, ranged reads, and explicit
write acknowledgement with `baseSha256`.

Also resolved here: head-of-line blocking (§4.6). One pipe is one queue, so a large response
would serialise ahead of a completion request and breach the interaction budget the whole
architecture is justified by. The source document asserted both the budget and the single pipe
and never connected them. Frame caps plus SFTP for bulk plus ranged reads address it without
abandoning the single control channel.

## A-B7 — Tauri v2 (2026-09-21)

**Decision.** Tauri v2.

**Rationale.** v1 is end of life; a new build targeting it would need migrating before
shipping. The source document referred to v2 in prose while showing a v1 manifest, which would
not validate.

## A-UI — Interface rendering: webview over the signed-off design system (2026-09-21)

**Status:** Decided 2026-09-21.

### Decision

The client renders its interface in the platform webview, consuming the signed-off HTML and
CSS design system directly. A-B7 (Tauri v2) stands. A native GPU-rendered UI layer in place of
the webview is rejected.

### Rationale

The design that carries stakeholder sign-off is an HTML and CSS artifact: a prototype page, a
stylesheet built on CSS custom properties, bundled webfonts, and a lint configuration that
enforces token use over raw hex and raw pixel values. Constitution Principle I makes that
artifact binding verbatim, and the Design System Compliance section requires the lint to run
in continuous integration.

A webview consumes that artifact as it stands. Any other renderer requires hand-reimplementing
the design system against a different primitive set, at which point "verbatim" is no longer
verifiable by anything but eye, and the adherence lint has nothing to lint. Principle I and a
native renderer are therefore not merely in tension — as written, they cannot both hold.

### Alternative rejected

**A single-process, natively GPU-rendered UI layer.** Proposed in a contributed architecture
document that has since been removed from the repository. It argued that a native renderer
would reduce per-keystroke latency relative to a webview.

Rejected on three grounds. It makes Principle I unsatisfiable, as above. The design carrying
sign-off would have to be re-authored as a native specification and put through fresh
stakeholder approval, which is design work not yet scheduled and not in anyone's plan. And it
arrived with a materially different product definition — a smaller local hardware budget and a
wider platform target — which contradicts §1.2 and §1.3 and would reopen decisions this
document has already closed.

Weighing evidence: the visual design has stakeholder sign-off; the contributed proposal had
none.

Its operational guidance is not lost with it. Starting language servers lazily on first use of
a language is already §7.3; bounded caches with explicit eviction are already §5.5; using LSP
and DAP for semantics rather than for editing is already §7.2 and §4.8.

### Reversal conditions

Two, either of which reopens this:

1. A future design sign-off delivers a native design specification rather than an HTML and CSS
   one. Principle I would then bind to that instead, and the reasoning above inverts.
2. Measurement against the interaction budget in §1.4 shows the webview cannot meet it on
   reference hardware. Principle V makes that a measured question, not an argued one, and
   F001's mock daemon harness is where the evidence would come from.

Absent either, this decision is settled and the alternative is not to be re-litigated.

## A-STATE — Interface session state lives outside the workspace cache (2026-09-21) — SUPERSEDED by A-STATE2

**Status:** Decided 2026-09-21. Promoted from `specs/001-app-shell/research.md`.

### Decision

Interface session state — window geometry, region layout, open document references and focus —
is persisted in its own JSON file in the platform application-data directory, not in the
SQLite workspace cache defined in §5.2.

### Rationale

The two have different lifetimes. The workspace cache is a disposable projection of a remote
source of truth, expected to be evicted (§5.5) and invalidated wholesale (§10.4). Interface
state is a durable user preference; losing it because a cache was cleared would be a defect.

The payload also has none of the properties that justify a database: no querying, no
concurrent writers, no partial reads, and no growth with workspace size. It is read once at
launch and written on change.

Practically, it also decouples the shell from F003, which sat behind `[OPEN: H-BOOT]` at the
time this was decided (resolved 2026-09-23 by A-BOOT). Had
session state lived in the workspace cache, the first feature in the build order would have
depended on a feature that cannot yet be built.

### Alternatives rejected

**The SQLite workspace cache (§5.2).** Rejected on the lifetime coupling above, and because
it would make F000 unbuildable until F003 completes.

**A store plugin.** A reasonable fit, but adds a dependency and a permission for what
serialisation and a path already do. Reconsider if the state grows to need migrations or
change notification.

**Platform-native preference stores.** Rejected because it splits one behaviour across two
implementations and two test paths for no user benefit.

### Reversal conditions

If interface state ever needs querying, cross-device synchronisation, or transactional
consistency with workspace data, revisit. None is currently in scope.

## A-E2E — End-to-end coverage is asymmetric across target platforms (2026-09-21)

**Status:** Decided 2026-09-21. Promoted from `specs/001-app-shell/research.md`.

### Decision

End-to-end tests run on Linux in continuous integration. macOS receives a scripted smoke
check — launch, await readiness, capture a screenshot, assert a clean exit — and is otherwise
covered by unit and integration tests.

This binds every feature with interface surface, not only the shell.

### Rationale

End-to-end driving of a Tauri application delegates to the platform's WebDriver
implementation. Linux provides one for its webview; macOS provides none for WKWebView. This is
a missing platform capability, not a configuration problem, so no amount of setup closes it.

The coverage loss is narrower than it appears: the interface layer is identical across both
platforms, so the same code is exercised either way. What genuinely differs is window geometry
behaviour and appearance, both reachable through integration tests and the screenshot check.

### Alternatives rejected

**Drive macOS through scripting or accessibility APIs.** A bespoke harness needing its own
maintenance, for a surface of two behaviours.

**Drop end-to-end everywhere for parity.** Rejected outright. Symmetry is not a reason to have
less coverage.

**Treat the smoke check as end-to-end.** Rejected as mislabelling. It proves the application
starts and paints; it exercises no user journey.

### Consequence for Principle VII

Constitution Principle VII requires end-to-end coverage and requires a recorded justification
for any omitted level. This decision is that justification, and it applies project-wide.
Individual features cite it rather than re-arguing it.

### Reversal conditions

A WebDriver implementation for the macOS platform webview, or a change of interface technology
that brings its own cross-platform driver.

## A-REQ — An in-flight request dies with its connection (2026-09-22)

**Decision.** When a connection is lost, every outstanding request resolves as
`ConnectionLost` before any reconnection attempt begins. Nothing is carried across, retried
automatically or replayed. The caller decides what to do next.

**Rationale.** The remote engine has no memory of a request issued on a connection that no
longer exists, so a "resumed" request would wait forever for a reply nobody will send.
Failing fast is both the honest report and the simpler implementation: there is no partially
valid state to reconcile on reconnect, because the connection owns everything keyed to it.

This binds every feature that issues a request. A caller that assumes its request survives a
blip will silently lose work — a save that reports success on send, an index run that
believes it completed. The rule is that a request outcome is always delivered, and
`ConnectionLost` is one of the outcomes.

**Rejected — replay on reconnect.** Requires the engine to deduplicate by request id across
connections, which §4.8 does not specify and which turns every non-idempotent method into a
correctness question. A `workspace/writeFile` replayed after a reconnect could overwrite a
change made in between.

**Rejected — hold requests until the connection returns.** Indistinguishable from a hang for
the user, and unbounded: a laptop closed overnight would wake with a queue of requests whose
purpose has expired.

### Reversal conditions

An engine-side session identity that survives a transport connection, with deduplication by
request id defined in §4.8.

---

## A-PRI — Outbound priority is stated by the caller (2026-09-22)

**Decision.** Two classes, `Interactive` and `Background`, with `Interactive` always written
first and FIFO within a class. The class is a parameter of the send call. Ordering applies
between frames, never within one: a frame already being written completes before anything
overtakes it.

**Rationale.** §4.6 assigns the ordering guarantee to the transport, and the transport cannot
infer the class. The same method is either class depending on why it was called —
`workspace/readFile` is interactive when the user opens a file and background when prefetch
warms the cache. Inferring from the method name would be wrong in exactly the case that
matters, which is the one where an indexing run is competing with a keystroke.

Two classes rather than five because nothing in this specification distinguishes more than
editor traffic from background work, and a priority scheme finer than its requirements is one
nobody applies consistently.

This binds every future caller: traffic sent without a stated class gets the default, and a
feature that sends bulk work as `Interactive` defeats the guarantee for everyone else.

**Rejected — inferring priority from the method name.** Wrong for the case above.

**Rejected — strict FIFO until a second traffic class exists.** §4.6 is normative and was
unassigned, and an unassigned normative requirement is how a thing quietly never gets built.

**Rejected — interrupting a frame in progress.** `Content-Length` has promised exactly that
many bytes follow; interrupting corrupts the stream for every subsequent frame. The 1 MiB cap
is what bounds the resulting delay.

### Reversal conditions

A traffic class that fits neither — a third party whose latency requirements sit between the
two — or measurement showing the 1 MiB cap admits an unacceptable head-of-line delay.

---

## A-OFFLINE — Offline editing with three-way merge (2026-09-23)

**Supersedes A-B3. Resolves `[OPEN: B3-reversal]`, and the B4 question that A-B3's existence
had made moot.**

**Decision.** Losing the connection leaves the editor writable. Edits persist locally against
the `baseSha256` the client held when the connection dropped. On reconnect the client performs
a three-way merge — base, local, remote — per file, fast-forwarding where the remote has not
moved and raising a conflict only for hunks that genuinely collide.

**Rationale.** Product judged offline editing a differentiator worth roughly two additional
features, which is the cost A-B3 priced and declined to pay on engineering grounds alone. That
was always product's call to make; A-B3 said so.

Three-way rather than last-writer-wins because the base hash already exists in the protocol:
`workspace/writeFile` carries `baseSha256` precisely so the engine can refuse a stale write
(§4.8, error `-32004`). Offline editing needs no new protocol field, only the merge and the
interface for it. Last-writer-wins would have been cheaper and is the one option that can
destroy a remote change without telling anyone — the failure this product cannot afford,
because the remote side is where CI and colleagues write.

Conflicts prompt rather than resolve automatically. A merge that silently picks a side is a
merge nobody can audit, and developers already have an accurate mental model for this from
version control.

**Consequences.** §11 is rewritten: the read-only lock is gone and reconciliation becomes a
merge rather than a pull. F012 `offline-readonly` grows from one feature to roughly three and
is renamed accordingly. A persisted outbox with base revisions becomes part of the cache
schema (§5.2). None of this touches the transport: A-REQ still holds, and an in-flight request
still dies with its connection.

**Rejected — last-writer-wins with a backup copy.** Silently loses the remote change unless
somebody notices a `.conflict` file. Defensible for a single developer whose remote never
changes underneath them; indefensible the moment CI or a second person writes.

**Rejected — refuse to sync and choose per file.** No merge engine and no silent loss, but it
discards work whenever a file changed on both sides for unrelated reasons, which is the
ordinary case rather than the exceptional one.

### Reversal conditions

Measurement showing developers essentially never edit offline, or a merge implementation that
proves unable to produce trustworthy results on the languages in scope.

---

## A-BOOT — Daemon deployment, version negotiation and mismatch policy (2026-09-23)

**Resolves `[OPEN: H-BOOT]`, which blocked every remote feature.**

**Decision.** Three parts.

*Deployment.* The client carries the engine binary and pushes it over the SSH connection it
already holds, into a per-user directory on the remote host. It does this on `EngineMissing`
(the exit-127 classification F001 already produces) and on a version mismatch. Integrity is
verified by comparing a hash of what landed against the hash of what the client shipped with;
a mismatch aborts without executing anything.

*Negotiation.* `auth/handshake` exchanges `protocolVersion` alongside the versions and
capabilities §4.8 already defines. The protocol version is an integer that increments on any
breaking change to §4.

*Mismatch policy.* The client is always the authority. If the engine's protocol version is
older, the client redeploys and re-executes it. If it is **newer**, the client refuses to
proceed and tells the user to update the client — it does not attempt to speak a protocol it
does not know.

**Rationale.** Pushing over the existing connection introduces no second trust root, no
package registry, no outbound internet requirement on the remote host, and no additional
credential. It works against any host the developer can already reach, which is the property
that makes the product usable against a machine the developer did not build.

The client being the authority is what makes skew always resolvable in one direction. Because
the client deploys the engine, the two can only diverge when the client is older — and the
remedy for that is updating one thing, which the user controls. A negotiation that tried to
find a common subset would need every version to know every other version's capabilities,
which is a compatibility matrix nobody maintains correctly.

Refusing a newer engine rather than attempting it is the same instinct as `Unknown` in F001's
failure classification: acting on a protocol you cannot verify produces confident wrong
behaviour, and a clear refusal is more useful than a subtle corruption.

**Rejected — remote downloads from a release URL.** Keeps large binaries off the SSH channel
and makes updates a URL change, but requires outbound internet from the remote host — which a
locked-down VPC will not have — and introduces a signing identity to verify, which the
delivery decisions have declined to establish.

**Rejected — pre-baked into the machine image.** Fastest first connect, nothing to deploy, but
every version bump rebuilds the image and the client becomes useless against any host the
developer did not build. That forecloses the ordinary case of pointing it at an existing
machine.

**Rejected — a version negotiation that finds a common subset.** The compatibility matrix
above.

### Reversal conditions

An engine that must run on hosts the client cannot write to, or a binary large enough that
pushing it over the control channel breaches the interaction budget during deployment.

---

## A-EC2 — Per-developer instances, stopped when idle (2026-09-23)

**Resolves `[OPEN: EC2]`.**

**Decision.** One instance per developer. Stopped automatically after 30 minutes without
interactive traffic. Wake target under 60 seconds from the developer's action to a usable
editor. Provisioning is per-developer and not shared.

**Rationale.** Single tenancy is the condition under which the authorization model in §16.3 is
acceptable rather than blocking — see A-SEC, which this decision closes. Sharing instances
would buy cost efficiency at team scale and immediately reopen authorization, per-client
resource limits and audit logging as blocking work.

Idle stopping rather than always-on because the usage pattern is a few hours a day against a
machine billed by the hour. The 30-minute threshold is long enough to survive a meeting and
short enough that a forgotten session costs one hour rather than a weekend.

The 60-second wake target is what makes idle stopping tolerable: beyond about a minute,
developers start leaving instances running to avoid the wait, which defeats the policy. Idle
detection keys on interactive traffic specifically — background indexing must not hold an
instance awake, or nothing ever stops.

**Rejected — always-on per developer.** Deletes wake latency as a design problem entirely, and
pays around the clock for a machine used a few hours a day.

**Rejected — shared instances.** Reopens SEC in full, as above.

### Reversal conditions

A team large enough that per-developer instances are the dominant cost, at which point SEC
must be answered properly before sharing anything.

---

## A-SEC — Authorization is SSH access, under single tenancy (2026-09-23)

**Resolves `[OPEN: SEC]`.**

**Decision.** Authorization is exactly the developer's SSH access to their own instance. No
additional authorization layer, no per-client resource limits, no audit log. The engine trusts
any client that authenticated over SSH, because under A-EC2 that client is the machine's only
user.

**Rationale.** SEC was never unconditionally blocking; its own entry says the model is
"acceptable single-tenant, blocking if shared". A-EC2 chose single tenancy, so this follows
from it rather than being decided independently.

Building an authorization layer over a single-user machine would add a second set of
credentials protecting a resource the first set already gates completely. The honest model is
that SSH access *is* the authorization, and saying so plainly is better than a token layer
that implies a boundary which does not exist.

**Rejected — build it anyway for future-proofing.** Speculative, and it would have to be
redesigned against whatever sharing model is eventually chosen, because the right boundary
depends on what is being shared.

### Reversal conditions

Any move to shared instances. This decision is downstream of A-EC2 and falls with it.

---

## A-SIGN — macOS ships unsigned, with documented bypass (2026-09-23)

**Resolves `[OPEN: SIGN]`.**

**Decision.** The macOS client is distributed unsigned and un-notarized. The download page
documents the Gatekeeper bypass.

**Rationale.** Product chose this over deferring macOS or paying for a Developer ID. It avoids
an annual cost and keeps CI free of signing secrets.

**Recorded cost, because it is real.** Every user's first launch is a security warning they are
instructed to dismiss. §3.9 refuses to train exactly that habit for host keys, on the grounds
that an IDE which teaches users to click through security warnings has removed the protection
entirely. This decision trains it at install time instead. The reasoning in §3.9 does not stop
being true because the warning comes from Gatekeeper rather than OpenSSH; the two are simply
being traded against different costs, and the trade is recorded here rather than left implicit.

Note also that A-UPDATE's Homebrew route does not rescue this: a cask installing an unsigned
application meets the same Gatekeeper prompt.

**Rejected — Apple Developer ID with notarization.** 99 USD per year and a signing identity in
CI. The only route to a first launch without a warning.

**Rejected — defer macOS entirely.** Would have closed the item as a scoping decision at no
cost, consistent with A-E2E already treating macOS as the reduced-coverage platform.

### Reversal conditions

A user base large enough that first-run friction costs more than the certificate, or an Apple
policy change that blocks unsigned applications outright rather than warning about them.

---

## A-UPDATE — Platform package managers deliver the client (2026-09-23)

**Resolves `[OPEN: UPDATE]`.**

**Decision.** The client is distributed through platform package managers — an apt repository
for Debian and Ubuntu, a Homebrew cask for macOS. No in-application updater.

**Rationale.** Developers update these the way they update everything else, and the package
manager owns the integrity checking, so there is no update feed to sign separately — which
matters given A-SIGN declined to establish a signing identity.

It pairs with A-BOOT rather than duplicating it: the client deploys the engine, so the client
is always the half that needs updating, and the package manager is how it gets updated. Version
skew always has exactly one remedy.

**Recorded cost.** This is three delivery pipelines to build and maintain — apt, Homebrew, and
whatever Windows eventually needs — for one client. An in-application updater would have been
one. That cost falls on F014.

**Rejected — built-in auto-updater.** One pipeline and the best experience for users who never
update by hand, but it needs signing keys for the update feed, which A-SIGN declined, and an
update server to run.

**Rejected — manual download with a skew warning.** Least machinery of all: the handshake
already detects the mismatch under A-BOOT, so the client could simply say so and link the
download.

### Reversal conditions

Maintaining the pipelines proving more expensive than the updater would have been, or a
platform target that has no package manager worth using.

---

## A-OBS — Local logs, plus a crash reporter that redacts (2026-09-23)

**Resolves `[OPEN: OBS]`.**

**Decision.** Structured logs stay on the developer's machine with bounded retention and are
never transmitted. Unhandled panics upload a stack trace and nothing else. No metrics, no
usage telemetry, no consent flow beyond the crash reporter's own opt-out.

**Rationale.** Local logging already exists and already has the property that matters: F001's
`FR-008` asserts a passphrase reaches no log, no error and no panic payload. Keeping logs local
means that assertion is the whole of the privacy story.

The crash reporter is the narrow exception, and it is worth it because unhandled panics are the
failures a developer will never report by hand.

**Hard requirement it creates.** A panic payload is precisely where a secret is most likely to
surface — F001 tested that case specifically, because `Secret`'s redaction is what stands
between a passphrase and a panic message. Any crash reporter must therefore redact before
transmission and must be tested against a sentinel, exactly as `a_passphrase_reaches_no_log_no_error_and_no_panic`
does today. An unredacted crash reporter would make a passing test a lie.

**Rejected — local logs only.** Simplest and strictly safest, with no transport to secure at
all.

**Rejected — opt-in anonymous metrics.** Would answer whether the interaction budget holds on
real networks rather than against the mock, which Principle V would value. Needs a collector, a
retention policy and a privacy statement.

### Reversal conditions

Evidence that the interaction budget behaves differently in the field than against the mock,
which would justify revisiting metrics with an explicit consent flow.

---

## A-PREVIEW — Previews are user-initiated (2026-09-23)

**Resolves `[OPEN: PREVIEW]`.**

**Decision.** The developer names the port and starts the preview explicitly. Nothing is
forwarded without being asked for. §9.3 is built on this.

**Rationale.** No heuristic to get wrong and no surprising port forwarding. The security story
is trivial precisely because there is no inference: a port is forwarded because someone asked
for that port. A-B2 already permits preview forwarding; this decides only how one starts.

Auto-detection remains addable later without breaking anything, because an explicit action is a
subset of an offered one.

**Rejected — auto-detected previews.** Nicer when it guesses right. Needs process or port
watching on the remote host, and a wrong guess forwards a port the developer did not intend to
expose.

### Reversal conditions

Usage showing developers start the same preview repeatedly by hand, which would make detection
worth its risk.

---

## A-WORKSPACE — Remote workspace identity and lifecycle (2026-09-23)

**Resolves `[OPEN: WORKSPACE]`.**

**Decision.** A remote workspace is identified by an opaque `workspaceId` minted by the client
and recorded on the instance. Its display name is the repository directory name and may
collide freely, because nothing keys on it. Re-opening a workspace that already exists on the
instance attaches to it rather than re-provisioning. Deletion is explicit and removes both the
remote directory and the local cache. No quota is enforced; the instance's disk is the limit,
and exhaustion surfaces as an ordinary engine error.

**Rationale.** §4.8 already makes `workspaceId` mandatory on every workspace method, so the
identity exists; what was undefined was how it is minted and what a second use does. Minting
client-side means a workspace is addressable before the engine has ever seen it, which is what
the first connect needs.

Names collide because names are for humans. Keying on a path or a name would make two
checkouts of the same repository indistinguishable, which is the ordinary case, not an edge
one.

No quota because under A-EC2 the instance is single-tenant: a developer filling their own disk
is a problem they can see and fix, and a quota would add a policy to enforce and a failure mode
to explain for no protection they need.

**Rejected — derive the identity from the remote path.** Makes re-burst and rename undefined,
and collides on exactly the common case.

**Rejected — enforce a per-workspace quota.** Meaningful only under sharing, which A-EC2
declined.

### Reversal conditions

Shared instances, which would make both naming collisions and disk quotas real problems.

---

## A-DAP — Debugging is out of scope for v1 (2026-09-23)

**Resolves `[OPEN: DAP]` by scoping it out rather than answering it.**

**Decision.** No debugging support in v1. The Debug Adapter Protocol, breakpoint synchronisation
and the variable model are not specified, not built, and not reserved for in the protocol.

**Rationale.** DAP is absent from the §19 build sequence, which is the honest signal that it was
never planned into this version. Specifying it now would be designing a large surface — adapter
lifecycle, breakpoint persistence across reconnects, a variable inspection model — with no
feature scheduled to consume it, and §4 would acquire methods nothing calls.

Scoping it out closes the open item honestly. An `[OPEN]` marker that means "we have not thought
about this yet" is indistinguishable from one that means "we decided not to", and the
difference matters to whoever reads this next.

**Rejected — specify it now.** Design work with no consumer, which would age badly before
anything used it.

**Rejected — leave it open.** Principle IV makes an open item a hard gate, so leaving it open
blocks a feature that does not exist for a capability nobody has scheduled.

### Reversal conditions

A decision to support debugging, which reopens this as a full design question and adds at least
one feature to the map.

---

## A-LSP — Language server resource and scope policy (2026-09-23)

**Resolves `[OPEN: LSP]`.**

**Decision.** One language server process per language per workspace, started lazily on first
use of that language (already §7.3). Each server runs under a cgroup memory limit, and a server
that exceeds it is killed and restarted rather than allowed to exhaust the instance. Multi-root
workspaces are not supported: one workspace is one root. Client capabilities are passed through
to the server unmodified.

**Rationale.** A memory limit is the only one of these that is load-bearing under A-EC2 — a
runaway server on a single-tenant instance takes down the developer's whole environment, and
restarting one server is recoverable where an out-of-memory kill of the engine is not.

Single-root because multi-root doubles the addressing model — every request would need to say
which root it means — for a case the product has not committed to. `workspaceId` already
distinguishes workspaces; a second level inside one is a different feature.

Capabilities pass through unmodified because the client is a thin renderer of LSP results. A
filtering layer would be a second place for capability bugs to live, and the failures would be
silent ones where a feature simply never appears.

**Rejected — a shared server across workspaces.** Saves memory and entangles the lifetimes: one
workspace closing would have to reason about another's state.

**Rejected — no memory limit.** The current state, and the reason this item was open.

### Reversal conditions

A product commitment to multi-root workspaces, or measurement showing per-workspace servers are
the dominant memory cost on a reference instance.

---

## A-IGNORE — One exclusion set, shared by indexer and watcher (2026-09-23)

**Resolves `[OPEN: IGNORE]`.**

**Decision.** Exclusions are the repository's own `.gitignore` files, plus a fixed built-in set
(`.git/`, `node_modules/`, `target/`, `dist/`, `build/`, `.venv/`, `__pycache__/`). One
resolved set is computed per workspace and used by both the indexer and the file watcher. No
per-workspace user configuration in v1.

**Rationale.** The item's own note says this "affects indexer and watcher agreement", and
agreement is the entire point: an indexer that indexes what the watcher ignores produces search
results for files whose changes are never noticed, which is worse than not indexing them.
Computing the set once and sharing it makes disagreement impossible by construction rather than
by discipline.

`.gitignore` because it is already there, already maintained, and already expresses precisely
"files this project does not consider its own". The built-in additions cover directories that
are routinely *not* in `.gitignore` yet ruinous to index — `node_modules` being the canonical
example.

No user configuration in v1 because the two sources above cover the real cases, and a third
source would need a precedence order that someone has to learn.

**Rejected — index everything and filter at query time.** Wastes the indexing cost and the
watch descriptors, which are the scarce resource.

**Rejected — a bespoke exclusion format.** A second thing to learn that duplicates what
`.gitignore` already says.

### Reversal conditions

A workspace where `.gitignore` and indexing needs genuinely diverge, which would justify a
per-workspace override with a stated precedence.

---

## A-D12 — Toolchain changes are pinned, verified, and fail closed (2026-09-23)

**Resolves `[OPEN: D12]`, recorded as "currently unpinned remote code execution".**

**Decision.** Any toolchain the engine installs or remediates is pinned to an exact version and
verified against a hash recorded in the client before execution. Verification failure aborts and
reports; it never falls back to an unverified copy. Nothing runs with elevated privileges — the
engine has the developer's own rights on their own instance and needs no more.

**Rationale.** The open item named the actual danger: unpinned remote code execution. "Install
the latest toolchain" means executing whatever a third party publishes, at a moment nobody
chose, on a machine holding the developer's credentials.

Pinning plus hash verification makes the installed artifact a decision recorded in the client
rather than a property of the network at install time. It is the same shape as A-BOOT's
integrity check, deliberately: one mechanism for "this binary is the one we meant", not two.

Failing closed rather than falling back because a fallback path is the one that runs when
verification fails — which is exactly when it must not run.

No privilege escalation because under A-EC2 the instance is the developer's own. A toolchain
that needs root on a single-user machine is a toolchain being installed in the wrong place.

**Rejected — install latest and verify nothing.** The status quo this item exists to end.

**Rejected — verify signatures instead of hashes.** Stronger in principle, and requires
establishing trust roots per toolchain publisher. Hashes recorded in the client are weaker
against a compromised publisher but need no key management, and the client is already the trust
root under A-BOOT.

### Reversal conditions

A toolchain with no stable artifact to pin, or a supply-chain requirement that demands
signature verification against a published key.

---

## A-TEST — The strategy already in force, made explicit (2026-09-23)

**Resolves `[OPEN: TEST]`.**

**Decision.** Unit tests for domain and application logic; integration tests against the mock
daemon for every transport-facing behaviour; end-to-end tests on Linux per A-E2E; an opt-in
suite against a real `sshd` for what no mock can prove. No feature is accepted without the
levels Principle VII requires, and any omitted level carries a recorded justification.

**Rationale.** This is not a new strategy — it is what F000, F018 and F001 actually did, written
down so it binds rather than being re-derived per feature. The item asked for "test strategy
beyond the mock daemon", and F001 answered it in practice: the mock proves the logic, the
opt-in `sshd` suite proves the invocation the mock cannot model, and the split is recorded in
that feature's quickstart.

The one addition worth stating as policy: a test that cannot fail is worse than no test,
because it reports confidence it has not earned. F001 found five such tests — a redaction check
reading a log that was never written, a priority check whose helper returned a constant, a
component test holding its own copy of the thing under test. Where a check guards a property
that would otherwise be invisible, verify the check fails when the property is broken.

**Rejected — a separate acceptance suite per increment.** A fourth level to maintain, when the
three above plus the opt-in suite already cover what acceptance would assert.

### Reversal conditions

A defect class that repeatedly escapes all four levels, which would indicate a missing one.

---

## A-NFR — How the §1.4 targets are measured (2026-09-23)

**Resolves `[OPEN: NFR]`.**

**Decision.** The §1.4 interaction targets are measured at the 99th percentile, at the boundary
between the interface and the transport, excluding any simulated network delay the harness
itself injects. A target is met when p99 is under it across at least 100 samples. Cold-start and
first-connect paths are excluded and measured separately.

**Rationale.** F001 set this precedent under SC-011 and it is generalised here. Wall-clock
measurement would have passed regardless of what the transport did, because the harness's own
250 ms round trip dominated it — so the measurement subtracts what the harness injects and
reports only what the system added. That distinction is the whole difference between a
performance gate and a number.

p99 rather than a mean because the interaction budget is about the keystroke that feels slow,
and a mean hides exactly those. p99 rather than max because a single scheduler hiccup should not
fail a build.

Excluding cold start because it is a different question with a different budget: first connect
involves deployment under A-BOOT and possibly an instance wake under A-EC2, and folding those
into a per-keystroke target would make the target meaningless.

**Requirement it creates.** A performance gate reports the measured value, not merely a verdict.
A budget that is only ever compared against tells nobody how much headroom remains, which is
what says whether the next feature's work can be afforded.

**Rejected — measure wall clock end to end.** Simpler, and dominated by whatever the harness
injects.

**Rejected — measure at the median.** Would pass a system that is slow exactly when it matters.

### Reversal conditions

Reference hardware changing enough to invalidate the targets themselves, which reopens §1.4
rather than this entry.

---

## A-BULKSIZE — The threshold at which a read leaves the control channel (2026-09-23)

**Refines A-BULK, which established the rule without fixing a number.**

**Decision.** A single read whose raw payload would exceed **512 KiB** is fetched beside the
channel, over its own `ssh` invocation on the existing control master. Reads at or under it travel
as `workspace/readFile` frames.

**Rationale.** §4.1 caps a frame at 1 MiB and §4.6 gives the reason: one pipe is one queue, so a
large response serialises ahead of every interactive request behind it. The threshold must
therefore sit below the cap with room for what the encoding adds, not at it. `workspace/readFile`
returns `{content, encoding, ...}`, and binary content is base64, which is four bytes out for every
three in. 512 KiB raw becomes roughly 683 KiB encoded, leaving about 340 KiB inside the cap for the
envelope, the path and the hash. A threshold of 768 KiB raw would encode to 1 MiB exactly and fail
on the first frame carrying a path.

This is also what reconciles two rules that read as if they conflict: that a large file must be
readable in ranges so the developer sees the beginning immediately, and that anything which would
not fit in a message must leave the channel. Both hold, because they govern different requests. The
*first screen* is a small ranged read and goes through the channel. The *whole file* is bulk and
goes beside it. §4.6 rule 3 describes exactly this.

**Rejected — set the threshold at the cap.** Off by the base64 expansion, so it fails on precisely
the files it was meant to permit.

**Rejected — route every file read through bulk.** One code path instead of two, and it puts a
second `ssh` invocation on the critical path of opening a small file. A-BULK measured that
invocation as free in *authentication* terms over an existing master; it is not free in
process-spawn terms, which is what a sub-250 ms open budget notices.

### Reversal conditions

A transport that multiplexes independent streams, which removes head-of-line blocking and with it
the reason for the threshold.

---

## A-CACHECAP — Content above 8 MiB is read but never cached (2026-09-23)

**Decision.** The client does not write content larger than **8 MiB** into the local cache. The
tree entry still exists, the file is still listed and navigable, and opening it fetches it every
time.

**Rationale.** Opening a multi-gigabyte artifact must not attempt to cache it whole, and "large" is
not a testable bound. A number is needed, and it is chosen against what the cache is *for*: §5.5 and
§5.6 describe a projection of source code, and §11.4's prefetch names manifests and recently changed
files. The largest real source files — generated parsers, vendored bundles, lock files — sit in the
low single-digit megabytes. 8 MiB clears them with room while excluding the artifacts that would
dominate disk for content nobody reads twice. Compressed at the budgeted ratio, one file at the cap
occupies about 4 MiB, so the cap also bounds what a single entry can cost.

**The consequence, stated rather than buried: a file above the cap is never available offline.** It
is reported as unavailable rather than shown empty, but a developer expecting a 20 MiB generated
file on a plane will not find it. That is the trade this number makes, and F012's offline editing
inherits it — the cap is the answer to "what can be edited offline at all".

**Rejected — no cap, cache everything read.** Simplest, and one clone of a repository with large
binaries fills the disk with content the fourteen-day window will not reclaim for two weeks.

**Rejected — a total-size budget with LRU eviction.** A better policy in the abstract, and it
contradicts §5.5, which specifies time-based retention, and A-WORKSPACE, which declined quotas on
the grounds that the developer's disk is theirs to manage. Changing that is a change to those
decisions, not a threshold choice.

**Rejected — derive the cap from free disk.** Makes behaviour depend on the machine, so the same
action caches on one laptop and not another, and no test can assert either.

### Reversal conditions

Evidence that real workspaces routinely hold source files above the cap, or the arrival of a
size-based budget in §5.5, which would replace this mechanism rather than tune it.

---

## A-DEADLINE — Interaction-path requests state their own limit (2026-09-23)

**Decision.** A request on the interaction path carries an explicit timeout derived from the §1.4
budget rather than taking the transport's 30 second default. The first instance is the hash
confirmation that precedes serving cached content, which uses **2 seconds**; on expiry the wait
ends, the developer is told the content could not be verified, and the cached copy is offered marked
unverified.

**Rationale.** The transport's default is sized for a request whose failure is an *error*. A
confirmation's failure is a *fallback*, and thirty seconds of a window that cannot be dismissed is
indistinguishable from the hang that showing the verification state exists to prevent.

2 seconds is eight times §1.4's 250 ms uncached target. Tighter values were considered and rejected
on a specific ground: a limit near the budget itself would expire routinely on ordinary
transcontinental latency under load, so the "unverified" marker would appear during normal operation
and developers would learn to ignore it — which costs more than the wait it saved. The marker has to
mean something.

**Rejected — take the transport default.** Free, and it makes a wedged engine look like a broken
application for half a minute.

**Rejected — set the limit equal to the interaction budget.** Correct as an expectation, wrong as a
deadline: budgets are p99 targets and deadlines must tolerate the tail the budget excludes.

**Rejected — no limit, with a cancel control.** Puts the work on the developer for a condition the
system can detect itself.

### Reversal conditions

Measured confirmation round trips whose p99 approaches the limit, which would mean the limit is
being set by the network rather than by the interface.

---

## A-BULK — Bulk data travels beside the protocol channel (2026-09-23)

**Decision.** Anything larger than a protocol message moves over its own `ssh` invocation
multiplexed on the existing control master, not through the JSON-RPC channel. §4.1 caps a frame
at 1 MiB, and the channel is for control traffic.

**Rationale.** F002's engine deployment is the first case and will not be the last: F003's file
reads, F017's artifacts and any future transfer face the same choice. Chunking a large payload
into 1 MiB frames would serialise it ahead of every interactive request on a single pipe, which
is the head-of-line problem A-B6 already solved by keeping bulk off the channel.

A second invocation costs nothing measurable. Measured against a real `sshd`: seven invocations
over one control master authenticate **once**, while three that bypass it authenticate three
more times. The master is what A-B1 bought, and this is what spends it.

One constraint the deploying side must respect: an invocation that would *create* the master
must not, because a master with `ControlPersist` backgrounds itself while holding the stdout
pipe it inherited — so reading that command's output waits forever for an EOF that cannot
arrive. Bulk invocations attach with `ControlMaster=no` and let the transport own the master.

**Rejected — chunk it through the protocol channel.** Head-of-line blocking, and it needs a
reassembly protocol §4 does not define.

**Rejected — a second authenticated connection.** Pays an authentication per transfer for
nothing, and doubles what a bastion sees.

### Reversal conditions

A transport that cannot multiplex, or a payload small enough that framing it is simpler than
invoking a second command.

---

## A-PROTOVER — The protocol version increments on breaking changes only (2026-09-23)

**Decision.** `protocolVersion` (§4.8) increments when something is removed, renamed, retyped,
or made required. Adding a method, an optional parameter or a result field does not increment
it. Both ends MUST ignore what they do not recognise.

**Rationale.** Without this rule, adding `session/onRestart` in F002 would have forced a
redeployment across every host in the estate for a notification an older client would simply
have dropped. A version that increments on additions is a version that makes every addition
expensive, and the predictable result is that people stop adding and start overloading what is
already there.

The rule is only safe because of the ignore requirement, which is stated as a requirement rather
than left as an implementation habit — it is the half that is easy to omit and impossible to
notice missing until an older client meets a newer engine.

This binds every feature that adds a method, which is most of them.

**Rejected — increment on every change.** Correct and useless: it turns an additive change into
an estate-wide migration.

**Rejected — semantic versioning with major and minor.** More expressive, and A-BOOT deliberately
made compatibility a single integer *compared* rather than negotiated. Two numbers invite a
compatibility matrix, which is the thing nobody maintains correctly.

### Reversal conditions

A change that is breaking for some methods and not others, which would mean the protocol has
grown independent parts and needs versioning per part rather than as a whole.

---

---

## A-WATCHSCOPE — Watching follows attention, not repository size (2026-09-24)

**Decision.** The engine watches only what the client asks it to. The client names **what it cares
about** — folder paths for expanded folders, file paths for open editor tabs — and the engine
derives the directories to watch: the folders, the parent of each named file, their ancestors up to
the workspace root, and the root itself, which is watched from registration and released only when
the workspace closes. §4.8 gains `workspace/watch` and `workspace/unwatch`, each taking a list,
each idempotent against a set the engine holds per workspace. §6.1's `watch()` changes shape to
match, and loses the `WatchHandle` that was never defined.

**Rationale.** §10.3 read as though the engine watched the whole workspace from registration and
the client merely filtered what arrived. That holds until the repository is large. Observing a
filesystem costs a resource the host limits per user, and a hundred thousand files can exhaust it
before the developer has looked at anything — so the failure appears as a watcher that is running
and silent, the one outcome §10.3 already forbids. Scoping to what is open makes the cost
proportional to attention, the same principle §10.1 applies to the tree.

The engine cannot know which folders are expanded or which files are open, so the scoping decision
forces a protocol method, and the catalogue had none: two file-event notifications and no request
to begin or end a watch, while §6.1 declared `watch()` on the provider trait. That is the fifth
absence of this kind, after `workspace/register`.

The client sends reasons rather than conclusions because a folder holding an open file is one path
with two reasons. Were the client to resolve that itself and send only directories, `unwatch` on a
collapse would be indistinguishable from `unwatch` on a tab close, and collapsing a folder would
silently stop reporting a file still open inside it. Sending both kinds keeps the arithmetic where
both facts are, and lets the engine recompute the watch set from the request at any time — which
is what makes reconnection one idempotent call rather than a replayed history. Watches do not
survive a dropped connection, and a client that resumed believing it was still being told about
changes would show a tree that had quietly stopped updating.

**Rejected — watch the whole workspace and filter client-side.** Needs no new method and is what
§10.3 read like. It exhausts host watch capacity on a large repository, and it puts client-side
filtering between a `node_modules` install and a flood on the control channel.

**Rejected — one `setWatched` call carrying the complete desired set every time.** Attractively
stateless, and it resends the entire set on every expand and collapse. The delta shape costs one
extra method and keeps the common message small, while the full set stays available for
reconnection precisely because the operation is idempotent.

**Rejected — notifications rather than requests.** Cheaper on the wire, and `refused[]` has nowhere
to go, so exhausted capacity becomes silence.

### Reversal conditions

Watch establishment measured outside §1.4's interaction budget, which would make the round trip per
expand the thing to remove: the call becomes fire-and-forget and refusals arrive as their own
notification. A feature needing recursive subtree watching, which adds a depth field to `paths[]`
rather than a third method.

---

## A-COALESCE — A hundred milliseconds per path, two hundred and fifty-six paths becomes wholesale (2026-09-24)

**Decision.** Repeated changes to one path collapse into one event on a **100 ms** trailing edge.
**256 distinct paths** changing within a rolling **1 second** window are delivered as a single
`workspace/invalidateAll` instead of individual events, and so is an `IN_Q_OVERFLOW` from the
kernel. `workspace/onFileEvent` carries an array, so one flush is one frame.

**Rationale.** §10.4 names the cases — branch switches, large pulls — and gives no number.
"Thousands" is not a bound a test can assert against, and "coalesce when there are a lot" is not
implementable. Both values are therefore derived from bounds rather than chosen.

The coalescing window is bounded above by the two-second reflection budget it sits inside, against
§18.1's modelled 250 ms round trip: at 100 ms the window is about a fifth of the transit in front
of it and a twentieth of the budget, leaving the measurement room to be a measurement. It is
bounded below by having to collapse anything at all — editors save by writing a temporary file and
renaming it over the target, two to three events per save, and below about 50 ms a burst survives
as a burst. The property that matters is that the number of events delivered is bounded by elapsed
time rather than by writes: at most ten per second per path, whatever the writer does.

The bulk threshold is bounded below by human action — editing touches single digits, a save-all in
a large project touches tens — so 256 sits an order of magnitude above anything normal work
produces. It is bounded above by the frame: §4.1 caps a frame at 1 MiB and A-BULKSIZE puts bulk
transfer at 512 KiB, and a path event serialises to roughly 150–250 bytes, so 256 of them is about
64 KiB, a factor of eight inside the smaller limit. That upper bound is only real because events
batch into one frame; while `onFileEvent` carried a single path the arithmetic described a message
shape the catalogue did not define, which is why the array is part of this decision rather than a
separate one.

A kernel queue overflow is routed the same way because it is the same problem. The queue is finite,
a burst that outruns the reader is dropped, and everything dropped is a change the client would
otherwise never hear about — the silence §10.3 and the watch requirements exist to forbid. The
client's response to a wholesale invalidation is already specified and already correct here: mark
the tree stale, re-read lazily, discard no cached content.

**Rejected — a window that widens adaptively under load.** This is "coalesce when there are a lot"
with arithmetic attached: the delivered event count stops being a function of elapsed time, and the
requirement stops being testable.

**Rejected — a leading-edge flush.** Reports the first change immediately, which reads better for a
single save, and debounces away the last write in a burst — the one whose content the developer
would actually fetch.

**Rejected — a byte threshold rather than a count.** Closer to the real constraint and harder to
reason about: the developer-facing question is whether something wholesale happened, which is a
count of paths, and a byte threshold makes the answer depend on path length.

**Rejected — reporting overflow as "cannot watch".** Watching has not failed and does not need
re-establishing, so the developer would be told they had lost freshness they still have.

### Reversal conditions

Measured p99 reflection approaching two seconds, which shrinks the window before anything else is
tuned. Ten events per second per path proving enough to delay interactive traffic under §4.6, which
widens it and re-derives the budget. A branch switch in a repository of realistic size producing
fewer than 256 changed paths — so the wholesale path never fires in the case it exists for — which
lowers the threshold. Overflow proving common enough to be costly, which sizes the reader's buffer
against the burst instead.

---

## A-UNPROVEN — An event marks content unproven, never valid and never invalid (2026-09-24)

**Decision.** An event naming a cached file sets an `unproven` flag beside the cache entry. It does
not change validity, does not discard the blob, and does not trigger a fetch. Validity remains a
comparison of two hashes and nothing else (§5.3).

**Rationale.** The cache's one rule is that a blob is valid exactly when its hash matches the
engine's, and F003 enforced it in the type system: `Validity` has a single constructor taking two
hashes, so no path exists by which anything other than a hash comparison can declare content valid.
An event is not a hash. Making "changed" a validity state would put a non-hash path into the one
type built to have none, and the check the client already performs before serving cached content
while connected does the work anyway — the flag is a hint that the existing comparison will
disagree, not a second mechanism.

Keeping the blob is the other half. Discarding it throws away content for a change the developer
may never open, and removes the copy they can still read offline. Marked possibly stale and
readable is strictly better than absent.

**Rejected — a third `Validity` variant.** The obvious shape, and precisely the reopening of the
invariant F003 closed deliberately.

**Rejected — deriving unproven from a modification timestamp.** No schema change, and it makes "has
this been disproved" a clock question. A cache that begins trusting clocks has stopped being
decidable.

### Reversal conditions

None foreseen. A second hint of this kind would make the two an explicit flags column rather than
accumulating booleans.

---

## A-WATCHLOCAL — Local mode does not watch in v1 (2026-09-24)

**Decision.** The local workspace provider returns `Unsupported` from `watch()`. No native watcher
ships for macOS or Windows, and no `inotify` integration ships in the client for Linux local mode.

**Rationale.** The product is a thin client against a remote engine; the local provider exists so
the read path can be exercised without a host. Watching it natively would mean FSEvents on macOS,
`ReadDirectoryChangesW` on Windows and a second `inotify` integration on Linux — three backends
serving a mode no requirement asks to watch.

The behaviour this produces is already specified rather than missing: losing the ability to watch
must not make a workspace unusable, browsing and reading continue, and the loss is stated. Local
mode therefore exercises the degradation path for free, and that path has a test either way.

**Rejected — one cross-platform watcher crate used by both binaries.** One dependency and all
platforms. Refused for the engine on binary size, which A-BOOT makes a first-class concern because
the engine is transferred on every first connect; refused for the client because nothing requires
it.

### Reversal conditions

A local-first mode with real users, or a cloud-burst feature needing a watched local workspace. The
port is already the seam, so either is a new adapter rather than a change to anything else.

---

## A-TASKLIFE — A running task outlives the connection that started it (2026-09-24)

**Decision.** A task keeps running when the client's connection drops. Its output is retained
while no client is attached, bounded by the same limit that bounds output for an attached one,
and a reconnecting client reattaches by task identity and receives what it missed.

**Rationale.** §7.3 already stops child processes on disconnect, and this decision departs from
that precedent deliberately. The precedent is about language servers: infrastructure the
developer never asked for, which restarts invisibly and costs nothing to lose. A build is
different in every respect that matters. The developer started it on purpose, it may be twenty
minutes in, and a dropped link is not a decision to abandon it. Stopping a language server on
disconnect loses nothing; stopping a build loses the work.

The client already has the identity it needs. §4.8 has the client choose `taskId` on
`execution/runTask`, so reattachment is a client that remembers what it started rather than a
discovery protocol. §15.2 anticipates the rest: the engine tracks active task IDs and PID
mappings so a client that returns reattaches rather than rebuilding its session by hand. That
section formerly claimed the same of a transient **crash**, and this record originally cited it
on that basis; the claim was wrong, because the map is memory and dies with the process. §15.2
has been narrowed accordingly. This decision is about a dropped connection, which the map does
survive, and it neither needs nor provides crash recovery.

**The consequence worth stating plainly.** This builds part of F020 `detached-engine` inside
F010. F020 owns surviving a disconnection, and a task that survives one is that, for tasks. The
alternative was to stop tasks now and let F020 change it later, which is the more conservative
sequencing — and it was rejected because it ships a known-wrong behaviour to preserve a feature
boundary, and because a developer losing a build to a wifi blip is a worse thing to ship than an
overlap two features can reconcile. F020's remaining scope is the engine itself and everything
that is not a task.

**Second-order consequence, for F005.** A-EC2 stops the instance after thirty minutes without
interactive traffic. A detached task produces no interactive traffic, so under that rule the
instance stops and the surviving task dies anyway, thirty minutes after the disconnect this
decision exists to survive. Whether a running task defers the idle stop is an idle-detection
policy, which F005 `ec2-lifecycle` owns explicitly. Recorded here rather than decided here,
because the trade is about billing a machine by the hour and belongs with the decision that set
the threshold.

**Rejected — stop tasks on disconnect, matching §7.3.** The narrow, reversible choice, and the
one that leaves F020 its whole job. Rejected because it makes a wifi blip cost a build, for the
whole interval until F020 ships, in exchange for a boundary that is an artefact of how the work
was divided rather than of how the product behaves.

**Rejected — hold the task for a grace period, then stop it.** Reads as a middle ground and is
not: holding a process for an absent client is the same machinery as keeping it, only
short-lived, so it pre-decides F020 exactly as much while adding a duration no requirement asks
for.

**Rejected — specify F020 first.** The cleanest sequencing. Rejected on cost: F020 depends only
on F002 and could have been built at any point, and reordering again would delay the feature
that makes F004 observable for a second time in one session.

### Reversal conditions

F020 arriving with a different model of survival, in which case this is the thing it reconciles
rather than a constraint it inherits. Or retained output for absent clients proving expensive
enough on a per-hour instance that a bounded hold beats an unbounded one — which is a number, not
a direction, and would narrow this decision rather than reverse it.

---

## A-TASKLIMIT — Tasks are bounded per process, not per tree, until a supervisor exists (2026-09-24)

**Decision.** An execution task runs in its own process group, and is constrained by per-process
resource limits inherited by its children. It is **not** placed in a cgroup. Full cgroup
isolation remains owed by whichever feature builds the shared process supervisor.

**Rationale.** The threat A-LSP names is precise: a runaway process exhausts the instance and the
out-of-memory killer takes the engine, "which is not recoverable" where restarting one server is.
What bounds that threat depends on the shape of the runaway, and on the size of the instance.

§1 puts the instance at 16 vCPU and 128 GB. At that size the realistic runaway is a **single
process** — a test with an allocation bug, a development server that leaks, a tool that never
frees. A per-process limit fits that exactly: the process is **refused further address space at
its ceiling**, in seconds and without anything else noticing. What it does then is its own — most
abort, and one that handles the failure may legitimately carry on. The limit denies the
allocation; it does not kill, and SC-026 measures the denial for that reason. The protection is
that the runaway cannot take the instance down, not that it dies.

The case a per-process limit cannot catch is a **tree** that collectively exhausts while every
member stays under its own ceiling — sixty-four compilers at three gigabytes each is a hundred
and ninety-two, and no single limit was exceeded. That is real, and on this hardware it takes
deliberate over-parallelisation to reach: `-j$(nproc)` is sixteen, and sixteen times three is
forty-eight of a hundred and twenty-eight. It is a flag pasted from a larger machine, not an
ordinary Tuesday.

So this decision buys the common case cheaply and leaves the uncommon one to the mechanism built
for it.

**Why not cgroups now.** Two reasons, and the second is the one that decided it.

A cgroup v2 subtree must be **delegated** before an unprivileged process can create anything in
it, which is a property of how the instance is provisioned. Provisioning is F005
`ec2-lifecycle`, which is unspecified and now sequenced after F010. Building against an
assumption about a feature that does not exist is how a gap at the edge of a decision becomes a
failure at the edge of an instance.

And the supervisor is shared. §7.3 describes one thing spawning language servers and tasks alike,
and F007 `lsp-multiplexing` needs the same isolation. Whichever feature builds it first defines
it for the other, and a subsystem designed against one caller's needs is one the second caller
reconciles rather than uses. That is an acceptable trade for a behaviour, as A-TASKLIFE was; it
is a poor one for a subsystem.

**Rejected — build cgroup v2 support in F010.** Catches every shape of runaway, and is what §7.3
and §15.4 describe. Rejected on the delegation dependency and on the shape of the overlap, above.

**Rejected — poll `/proc` and kill a tree past a threshold.** Bounds a tree without privileges or
delegation, which is more than per-process limits manage. Rejected because it reacts at the poll
interval, so a fast allocator crosses the threshold and keeps going between samples, and because
it is custom machinery no other system runs — the bugs in it would be entirely ours, bought to
cover a case that needs deliberate misuse to reach.

**Rejected — nothing beyond the process group.** Leaves the threat A-LSP names unmitigated for
tasks and relies on the out-of-memory killer choosing the right victim, which it usually does and
not always.

### Reversal conditions

A tree exhausting the instance in practice rather than in principle — at which point the
mechanism is cgroups and the question is only who builds it. F005 specifying provisioning in a
way that guarantees a delegated subtree, which removes the dependency this decision avoided. Or
F007 building the shared supervisor, which is where the isolation was always owed; this decision
then narrows to the process group, and the per-process limits become redundant rather than wrong.

---

## A-TASKSTREAM — A terminal merges the streams; separating them costs the terminal (2026-09-24)

**Decision.** `execution/runTask`'s `pty` parameter chooses between two output shapes, and the
choice is exclusive. With `pty: true` the task has a pseudo-terminal, `isatty` is true, and output
arrives merged on `execution/onStdout`. With `pty: false` the task has separate pipes, `onStdout`
and `onStderr` are distinguishable, and `isatty` is false.

**Rationale.** This is not a preference. A pseudo-terminal is **one device**, and a process whose
standard output and standard error are both attached to it writes both into the same stream —
which is what a terminal is, and why `2>/dev/null` exists. Two requirements that each look
reasonable alone cannot both hold for one task: a process must believe it has a terminal, and its
two streams must be separable.

§4.8 anticipated it without saying so. The catalogue gives `runTask` a `pty` parameter *and*
defines both `onStdout` and `onStderr`, and the only reading under which all three facts are
consistent is that the parameter chooses. This record states what the catalogue implied.

The consequence is one a caller chooses rather than suffers. A terminal panel takes the first
shape and gets a real terminal. A caller that wants to parse a build's diagnostics takes the
second and gets separation, at the price of the process no longer colouring its output or drawing
progress — the same price every continuous integration system pays for the same reason.

**Rejected — one pseudo-terminal per stream.** Possible and behaviourally wrong: programs expect
their two streams to share a terminal, so `isatty` would be true on both while a resize applied
to one of them.

**Rejected — a terminal for output and a pipe for errors.** Produces a state no real terminal
produces, where a process sees a terminal on one descriptor and not the other, and no program is
written against it.

### Reversal conditions

None foreseen. This is a property of the mechanism rather than a choice about it, and the only
thing that would reverse it is a terminal abstraction that is not one device.

---

## A-TASKEXEC — An engine re-execution terminates tasks and reports them (2026-09-24)

**Decision.** Before the engine replaces its own binary and re-executes (§15.3), it terminates
every running task using the same escalation `execution/terminate` uses, and names each one in the
`unpreserved` list of the `session/onRestart` notification that follows. Tasks do **not** survive
a re-execution.

**Rationale.** A re-execution replaces the process image. The task set is memory and the
pseudo-terminal descriptors are close-on-exec, so both are gone the moment `exec` succeeds — while
the child processes are not gone at all. They keep running, still children of the same pid, now
watched by nothing and reachable through nothing the protocol exposes. That is §7.3's FR-025
prohibition reached through a supported operation rather than through a failure, and it is the
worst of the three available outcomes because it is invisible: the developer sees the engine come
back healthy and never learns that a build is still burning CPU with no way to stop it.

**What this needs that does not exist yet.** The ids are drained by the **old** image and
`session/onRestart` is emitted by the **new** one, and the only thing crossing the `exec` today is
`APEX_SESSION_ID`. `SessionRegistry::new()` hardcodes `unpreserved: Vec::new()` in both branches,
so a straight reading of this decision produces an empty list and reports nothing — it would look
implemented and deliver none of its value. The terminated ids travel the way the session identity
already does, as a second environment variable: a channel proven across exactly this boundary,
needing no new mechanism.

The mechanism this decision uses already existed and was already addressed to this feature.
`session/onRestart` carries `unpreserved` so the client can tell the developer what a restart
cost, and `engine/src/session.rs` has carried the comment "F007 and F010 will have something to
report here" since F002. F010's entry in that list is the task set. The decision is less a choice
of mechanism than the discovery that the mechanism had been waiting for its second caller.

**The alternative, and why it was rejected.** Descriptors can be carried across an `exec` by
clearing `FD_CLOEXEC` and passing the identity-to-pid map through the environment, the way
`APEX_SESSION_ID` already travels. That would let a build survive an engine update, which is
strictly better for the developer in the moment. It was rejected because it makes every future
change to the task set a compatibility problem between two versions of the engine — the image
that opened the descriptors and the image that inherits them — for a benefit available only
during an update the developer did not ask for and does not observe. A terminated task the
developer is told about is a smaller harm than a surviving task whose owner and format are
negotiated across a version boundary.

**Reversal conditions.** Two, either of which is sufficient. First, if the engine ever holds its
task map outside its own process image — a supervisor process, or a small on-disk record of
identity-to-pid — then carrying tasks across a re-execution stops requiring descriptors to survive
an `exec` and the compatibility objection disappears with it. Second, if updates become frequent
enough that losing a build to one is a routine cost rather than a rare one, the balance inverts:
this decision is priced on an update being something a developer does occasionally and does not
watch.

**What this costs, stated plainly.** A developer whose twenty-minute build is running when the
engine updates loses it. The mitigation is not in this record: an update is a client-initiated
operation (§3.8), so a client that declines to update while tasks are running would avoid the
cost entirely. That is a client policy and belongs with whichever feature owns update scheduling,
not here.

## A-TERMPALETTE — A terminal needs sixteen colours; the system defines three (2026-09-24)

**Decision.** The three semantic hues the prototype states — `#7fa98f` success, `#d4736a` error,
`#c9a96a` warning — are extracted into design tokens by `ds-sync` like any other prototype value,
and the terminal is themed with them. The remaining ANSI colours come from the terminal library's
own palette, as a **named, recorded exception** rather than a silent one. A full sixteen-colour
ramp is owed to the design system and is not F010's to invent.

**Rationale.** A terminal renders sixteen ANSI colours plus a default foreground and background.
The signed-off design system defines two accent ramps, a nine-step neutral ramp and structural
colours — no red, green, yellow, blue, magenta or cyan. The prototype's terminal uses three hues
and states them as raw hex in its own markup, which makes those three extractable on exactly the
grounds every layout token was extracted. Blue, magenta, cyan and the eight bright variants have
no source anywhere in the signed-off material.

That leaves three routes and one of them is honest. Inventing thirteen colours puts a designer's
decision in an engineer's commit, which is what Principle I exists to prevent, and would be the
largest unreviewed addition to the design system to date. Extending the prototype is the correct
act, but it is a design act and not this feature's. Using the library's palette for what the
system does not define is smaller than either, reversible in one file once the ramp exists, and —
the deciding point — it is the only one of the three that leaves a visible marker saying a
decision is still outstanding.

**Reversal conditions.** One, and it is expected rather than hypothetical: the design system
gaining a sixteen-colour ANSI ramp. When it does, `palette.ts` maps every slot to a token, SC-016
widens back to its original wording, and this record is superseded rather than amended. A second,
weaker condition: if a second surface ever needs ANSI colours — a diff viewer rendering coloured
output, a log panel — the cost of not having the ramp is paid twice, and the argument for treating
it as owed rather than urgent weakens accordingly.

**The consequence, stated plainly.** SC-016 was written as "zero raw colour values" and has been
narrowed: it now measures that the three hues the system defines are taken from tokens, and
records the library's default palette as the accepted source for the rest. A criterion asserting
zero raw values while thirteen of sixteen colours have no token to use is unmeetable, and the
failure mode of an unmeetable criterion is that somebody satisfies it by inventing the tokens —
which is the outcome this record exists to prevent.

## A-STATE2 — The durable client store also carries task identities (2026-09-24)

**Supersedes A-STATE (2026-09-21)**, which is otherwise unchanged and remains the record of why a
durable client store exists and what shape it takes.

**Decision.** The client's durable store carries, in addition to A-STATE's window geometry, region
layout, open document references and focus, the **identities of tasks this client started** and the
workspace each belongs to. Nothing else about a task is stored: no output, no environment, no
command.

**Rationale.** A-TASKLIFE makes a task outlive the connection that started it, and `execution/attach`
reaches one by an identity the client must already know. A client that restarts therefore needs its
identities to have survived the restart, and A-STATE's enumerated payload does not include them —
so F010 either extends that payload or reattachment works only for a client that never closed.

Recorded as a new record rather than an edit to A-STATE because Principle III says records are
dated and superseded, never edited in place, and because the two decisions have different owners:
A-STATE is F000's, made about a shell's own state, and this is F010's, made about work running
somewhere else.

**Why not more than the identities.** Storing a task's command would put a credential passed in
argv on disk, which FR-005a's accepted boundary does not extend to; storing output would make the
store grow without bound for a client that never returns. The identity is the smallest thing that
restores reachability, and `execution/list` covers the client that has lost even that.

**Reversal conditions.** If task identities ever become discoverable without client state — which
`execution/list` already makes true for a client that can enumerate — the stored copy becomes an
optimisation rather than a requirement, and a client that prefers not to persist anything could
drop it. It is kept because enumeration costs a round trip at startup and the stored identity does
not.

## A-WSCLOSE — What closing a workspace means, precisely (2026-09-24)

**Decision.** Three answers to questions §4.8's `workspace/close` row leaves open. The response is
written once every task of that workspace has been **signalled**, and each end is reported by its
own `execution/onExit` as usual. Closing **deregisters** the workspace, being
`workspace/register`'s counterpart. A second close of an already-closed workspace is **`-32001`**,
not an idempotent success.

**Rationale.** Each closes a genuine alternative, which is why they belong here rather than in a
contract. Recorded late: they were taken while `contracts/task-methods.md` was written, and stating
them there left three decisions with rejected alternatives outside Appendix A, which Principle III
does not allow.

Answering after every task is **signalled** rather than after every task has **ended** is a
correction to this record's first version, which said the latter. Ending takes up to the
five-second escalation, and the use case runs on the engine's single dispatch thread — the same
thread that reads the client's stdin. Waiting there would mean five seconds in which no keystroke,
resize or cancellation is so much as read off the pipe, which is §1.4 and FR-012 failing through
the mechanism meant to satisfy FR-024. There is no deferred reply to fall back on: an `Action` is
a reply, nothing, or a restart.

SC-013 stays checkable without it. "Closing a workspace leaves zero of its tasks running" is
observed through each task's `onExit`, which is a defined event with a defined order, rather than
through a response whose timing hid the wait. A test waits for N exits, not for a sleep. The
escalations still run concurrently across the workspace's tasks, so closing ten costs five seconds,
not fifty.

Deregistering follows from being `register`'s counterpart: a close that left the id registered
would leave the engine holding a canonicalised root for a workspace the client has finished with,
and the client would have no way to say so.

Refusing a second close departs from FR-019, where terminating an already-terminated task succeeds.
The two look alike and are not. FR-019's race is a client racing an end **the engine decided** — the
task exited on its own — and reporting that as a failure would make a correct client look broken.
A workspace never closes itself, so a second close means the client has lost track of its own
state, and telling it so is a service.

**Reversal conditions.** If a client is ever expected to close a workspace it may not have opened —
a supervisor tidying up after a crash, say — then refusing the second close becomes the unhelpful
answer and idempotent success becomes right. Under A-EC2's single tenancy and one client per
engine, no such caller exists.

## A-E2ESCOPE — Which acceptance scenarios owe an end-to-end test (2026-09-25)

**Decision.** Principle VII's "each one MUST have a corresponding automated test" is read
**loosely**: the corresponding test must exist at the level where the scenario's substance is
**observable**, which is end to end for most scenarios and is not end to end for all of them. A
scenario covered below the end-to-end level carries a **written justification in its own feature's
specification**, naming the level that covers it and the property that is not observable through a
driven interface. The justification is per scenario, not per feature and not per level.

**Rationale.** The sentence is genuinely ambiguous and both readings are defensible. It sits under
the **End to end** bullet, which is the strict reading's whole case; it says "a corresponding
automated test" rather than "a corresponding end-to-end test", which is the loose reading's. A
principle that can be satisfied two ways satisfies neither until someone writes down which, and
Principle III says the writing down happens here.

The strict reading fails on a class of scenario this project has several of, where the property
under test is an **absence** and the interface cannot show it. F010's FR-005a is the clearest: a
task's environment must never reach a log or a crash report. A driver can observe a terminal panel
showing output; it cannot observe a core dump that was not written, because `RLIMIT_CORE = 0` means
there is no artifact to inspect and the passing state is that nothing exists. An end-to-end test
written for it would assert something adjacent — that the app still runs, that the panel still
scrolls — and pass whether or not the property held. That is a test that cannot fail, which
Principle VII's own rationale rejects in its last sentence, and which this project has already
produced five of.

Per **scenario** rather than per **level** is the operative part, and it is where this record adds
something the constitution does not already say. Principle VII's closing paragraph permits omitting
a level with a one-line justification naming why the feature has **no surface** there. That is an
all-or-nothing instrument: a feature either has end-to-end surface or it does not. F010 has plenty
— a build runs, its output appears, a keystroke interrupts it — alongside a handful of scenarios
that have none. Under the strict reading F010 cannot use the omission clause honestly, because the
level is not absent, and so it would owe an end-to-end test for every scenario including the ones
where that test would be theatre.

**Alternatives rejected.** *Strict, with the omission clause used per feature* was rejected above:
it forces a false statement, since the feature does have end-to-end surface. *Strict, with no
escape* was rejected because it buys its rigour with tests that pass unconditionally, which is worse
than the gap it closes — an unconditionally passing test is a claim of coverage that is not true,
and it is durable, because nothing ever fails to prompt a second look. *Loose with a blanket
per-feature justification* was rejected because a blanket justification is the thing that decays: it
is written once, and then every later scenario shelters under it without anyone re-asking whether it
applies. Requiring the sentence next to the scenario keeps the cost proportional to the number of
exemptions, which is the only pressure that keeps the number small.

**Consequences.** A feature specification's acceptance scenarios acquire a third state. A scenario
is either covered end to end, or covered lower with a named level and a named reason, and a scenario
with neither is an incomplete specification that `/speckit-analyze` should surface. The justification
names a level that must actually contain the test; "covered by unit tests" without one is the
blanket form this record rejects.

**Reversal conditions.** If the exemptions stop being a handful — if a feature's justifications
outnumber its end-to-end tests — the loose reading has become the default rather than the exception
and the pressure this record relies on has failed. The remedy then is not to tighten the wording but
to ask why so much of that feature is unobservable through its own interface, which is usually a
statement about the interface rather than about the tests.

# Appendix B — Open Items

**All items resolved 2026-09-23.** Nothing here blocks a feature. The table is kept as a record
of what was open and where each was answered, because a resolved question and a question nobody
asked look identical once the marker is gone.

Constitution Principle IV makes an `[OPEN: id]` marker a hard gate on the feature it appears in,
and notes that the feature map's sequence gate cannot see specification holes — it reported F002
ready while H-BOOT, which defines most of F002, was undecided. That is the situation this round
closed.

## Product decisions

| Id | Question | Resolved by |
|---|---|---|
| **B3-reversal** | Is offline editing a product differentiator worth roughly two features? | **A-OFFLINE** — yes; A-B3 superseded, three-way merge on reconnect |
| **PREVIEW** | Auto-detected previews, or user-initiated? | **A-PREVIEW** — user-initiated; nothing forwarded unasked |
| **EC2** | Idle threshold, cost target, wake latency, provisioning, per-developer or shared instances | **A-EC2** — per developer, stopped after 30 min idle, under 60 s wake |
| **WORKSPACE** | Remote workspace naming, collision, re-burst, deletion, quota | **A-WORKSPACE** — opaque id, names may collide, re-open attaches, no quota |

## Engineering decisions

| Id | Question | Resolved by |
|---|---|---|
| **H-BOOT** | Daemon deployment, version negotiation, mismatch policy | **A-BOOT** — client pushes over SSH, client is the authority, refuse a newer engine; see also **A-BULK** and **A-PROTOVER**, promoted from F002 |
| **DAP** | Debugging protocol, breakpoint sync and persistence, variable model | **A-DAP** — out of scope for v1, scoped out rather than answered |
| **LSP** | cgroup limits, multi-root workspaces, capability passthrough | **A-LSP** — memory-limited per language per workspace, single root, passthrough |
| **IGNORE** | Indexing and watch exclusion set, per-workspace configuration | **A-IGNORE** — `.gitignore` plus a built-in set, one set shared by both |
| **D12** | Safe toolchain remediation: pinning, verification, failure path, privileges | **A-D12** — pinned, hash-verified, fails closed, no privilege escalation |
| **SEC** | Authorization beyond SSH access, per-client limits, audit log | **A-SEC** — SSH access is the authorization, downstream of A-EC2 |

## Delivery decisions

| Id | Question | Resolved by |
|---|---|---|
| **SIGN** | Code signing and notarization | **A-SIGN** — unsigned with documented bypass; cost recorded against §3.9 |
| **UPDATE** | Client update mechanism | **A-UPDATE** — platform package managers, no in-app updater |
| **TEST** | Test strategy beyond the mock daemon | **A-TEST** — the strategy F000/F018/F001 already follow, made binding |
| **OBS** | Metric names, transport, retention, telemetry privacy | **A-OBS** — local logs, crash reporter that must redact |
| **NFR** | Percentiles, measurement points, exclusions for the §1.4 targets | **A-NFR** — p99 at the interface boundary, harness delay excluded |

## Adding an open item

A new `[OPEN: id]` marker goes in the body at the point it blocks, and a row goes here naming
what it blocks. It is removed only by a decision recorded in Appendix A, never by deciding it
inline in a feature specification — a decision that binds several features belongs where all of
them can find it.

## Provenance

The source of this specification was an LLM-authored architecture narrative. It carried
generation scaffolding into several sections, named a Rust crate that does not exist, included
a sample that did not compile, and specified two different schemas and two different protocol
contracts. All of that has been corrected or removed.

The working assumption for anyone extending this document: named crates, flags and figures are
claims to verify, not decisions already validated.
