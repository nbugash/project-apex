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
|    +-- Process supervisor ... build/test/run, cgroup-isolated    |
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
| -32006 | Task not found or already exited |
| -32007 | Payload exceeds the frame limit |
| -32008 | Request cancelled by the client |

Every error carries a human-readable `message`. Errors that a user can act on carry a `data`
object describing the remedy.

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

Outbound frames are priority-queued: LSP and editor traffic ahead of background work such as
prefetch and indexing status.

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
| `workspace/readDirectory` | request | `workspaceId`, `relativePath` | `items[]` of `{name, type, size, modified}` |
| `workspace/stat` | request | `workspaceId`, `relativePath` | `{type, size, modified, sha256}` |
| `workspace/readFile` | request | `workspaceId`, `relativePath`, `offset?`, `length?` | `{content, encoding, sha256, totalSize}` |
| `workspace/writeFile` | request | `workspaceId`, `relativePath`, `content`, `baseSha256` | `{sha256}` |
| `workspace/createFile` | request | `workspaceId`, `relativePath` | `{sha256}` |
| `workspace/createDirectory` | request | `workspaceId`, `relativePath` | — |
| `workspace/rename` | request | `workspaceId`, `fromPath`, `toPath` | — |
| `workspace/delete` | request | `workspaceId`, `relativePath`, `recursive` | — |
| `workspace/search` | request | `workspaceId`, `query`, `maxResults` | `matches[]` |
| `workspace/onFileEvent` | notification | `workspaceId`, `event`, `relativePath`, `toPath?` | — |
| `workspace/invalidateAll` | notification | `workspaceId` | — |

`workspaceId` is mandatory on every workspace method. The original "formal" contract omitted
it, which silently removed multi-workspace addressing.

`encoding` is `utf8` or `base64`. Binary files are legal and are returned base64-encoded within
the frame limit, or fetched over SFTP when larger.

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
| `execution/runTask` | request | `workspaceId`, `taskId`, `command`, `cwd`, `env`, `pty` | `{pid}` |
| `execution/writeStdin` | notification | `taskId`, `data` | — |
| `execution/resizePty` | notification | `taskId`, `cols`, `rows` | — |
| `execution/terminate` | request | `taskId`, `signal` | — |
| `execution/onStdout` | notification | `taskId`, `data` | — |
| `execution/onStderr` | notification | `taskId`, `data` | — |
| `execution/onExit` | notification | `taskId`, `exitCode`, `signal?` | — |

`writeStdin`, `resizePty`, `terminate` and `onExit` did not exist in the original contract,
which made the integrated terminal write-only and left no way to stop a runaway process or
learn that a build finished.

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
```

Three corrections are load-bearing:

- **`file_id` is an opaque UUID**, not a hash of the path. Path-derived identity meant a rename
  produced a new identity and orphaned the cached blob. Renames now update `relative_path`,
  `parent_path` and `name` in place, and the cached content survives.
- **`last_accessed_at` exists.** The eviction policy (§5.5) evicts blobs unopened for a period,
  which the original schema could not express because it stored only write time.
- **`files_fts` exists.** Offline path search was specified as a leading-wildcard `LIKE`, which
  cannot use an index.

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
    async fn watch(&self, path: &RelPath) -> Result<WatchHandle>;
}
```

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
orchestrator.

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

The engine owns all file watches, using `inotify` scoped to the workspace with build and
dependency directories excluded.

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
installation. Tasks run in a local PTY via `portable-pty` against the user's shell. The SQLite
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

It runs child processes under cgroups (§7.3) so no single language server or build can
destabilise it.

## 15.2 Availability

Against the 99.9% target (§1.4), the engine tracks active task IDs, PID mappings and language
server session state, so a transient crash can be recovered rather than requiring the developer
to rebuild their session by hand.

## 15.3 Updates

The engine supports in-place binary replacement and re-execution so toolchain updates do not
require the developer to intervene. This makes client/engine version skew a routine condition
rather than an exception, which is why §3.8 is blocking rather than cosmetic.

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
   developer's own user. This is expected — it is what a build is — and is bounded by cgroups
   and by the instance being developer-owned, not shared.
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

## A-STATE — Interface session state lives outside the workspace cache (2026-09-21)

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

---

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
| **H-BOOT** | Daemon deployment, version negotiation, mismatch policy | **A-BOOT** — client pushes over SSH, client is the authority, refuse a newer engine |
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
