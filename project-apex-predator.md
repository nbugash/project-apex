# Project Apex Predator — System Specification

**Status:** Active. This document is the source of truth for Project Apex Predator.
**Revision:** 2026-09-21.
**Supersedes:** the original architecture narrative (preserved at `project-apex-predator.md.bak`).

## How to read this document

Statements here are normative. Where a design decision was made deliberately, it is marked
**(A-xx)** and its rationale, alternatives and rejection reasons are recorded in Appendix A.
Where something is genuinely undecided it is marked `[OPEN: id]` and listed in Appendix B;
those markers are blocking for the feature they appear in.

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

`[OPEN: NFR]` This budget names no percentile, no measurement point and no excluded conditions,
and the same is true of the availability and footprint targets below. They are directional until
Appendix B, NFR is closed.

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

`[OPEN: H-BOOT]` The invocation in §3.1 assumes `/usr/local/bin/ide-engine` exists at the
expected version. How it arrives, how its version is verified against the client, and what
happens on absence (`ssh` exits 127) are unspecified. This is a prerequisite for every
remote-mode feature and is made load-bearing by the in-place binary replacement in §15.3. See
Appendix B, H-BOOT.

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
| `auth/handshake` | request | `clientVersion`, `capabilities` | `engineVersion`, `protocolVersion`, `capabilities` |
| `session/shutdown` | request | — | — |
| `log/onMessage` | notification | `level`, `message`, `source` | — |

`[OPEN: H-BOOT]` Behaviour on protocol version mismatch is undefined. See Appendix B.

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

`[OPEN: LSP]` Concrete cgroup limits, multi-root workspace handling, and capability negotiation
passthrough are unspecified. See Appendix B.

## 7.4 Debugging

The engine acts as a DAP broker, fronting `delve` (Go), `lldb-vscode` (Rust, C, C++, Zig) and
`debugpy` (Python).

`[OPEN: DAP]` Beyond this intent, debugging is unspecified: no methods, no breakpoint
synchronisation, no variable inspection model, and no persistence for breakpoints. This is a
whole feature that the original document named without defining. See Appendix B.

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

`[OPEN: PREVIEW]` Whether previews are auto-detected — the engine notices a process binding a
port and offers a banner — or user-initiated is undecided. Auto-detection is better UX and
requires watching listening sockets; user-initiated is trivial and cannot surprise anyone. It
does not affect the transport. See Appendix B.

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

`[OPEN: IGNORE]` The exclusion set is unspecified. At minimum it should cover `node_modules`,
`target`, `.git` internals, `build`, `dist` and `__pycache__`, but it needs to be configurable
per workspace and shared with the indexer so both agree on what is invisible. See Appendix B.

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
2. Re-run the handshake and verify protocol compatibility (`[OPEN: H-BOOT]`).
3. Reconcile: compare cached hashes against the engine for open files; refetch what changed.
4. Restore language servers for open workspaces.
5. Unlock the editor and update the status bar.

Because nothing was written offline, reconciliation is a pull. There is no merge, no conflict
prompt and no data to lose — which is the main thing §11.1 buys.

`[OPEN: B3-reversal]` Conflict resolution exists as a problem only if §11.1 is reversed. Under read-only
offline it is not needed. Should the outbox model be adopted, this section requires a stored
base revision in `file_contents`, a three-way merge, and a conflict UI.

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

`[OPEN: D12]` Automated installation is specified in intent but not safely. As described it
runs unpinned network installs (`go install ...@latest`, `brew install`) triggered by a UI
button, with no version pinning, no verification of what is fetched, no failure path, no
privilege model and undefined offline behaviour.

Before this ships it requires: pinned versions in the dependency manifest, verification of
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

`[OPEN: WORKSPACE]` Remote workspace registration is unspecified: naming and collision
behaviour under `~/ide-workspaces/<name>`, whether a second burst of the same project
overwrites or forks, deletion, and disk quota. See Appendix B.

## 15.5 Instance lifecycle

`[OPEN: EC2]` Substantially unspecified. The intent is proactive stop on sustained
disconnection with dynamic address resolution on wake, and security groups restricted to port
22. Undefined: the idle threshold before stopping, the cost target this serves, acceptable wake
latency, how instances are provisioned in the first place (AMI contents, infrastructure as
code), and whether an instance is per-developer or shared. Every one of these changes the
feature. See Appendix B.

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

`[OPEN: SEC]` There is no authorization model beyond "whoever can SSH to the instance", no
per-client resource limits, and no audit log of what the engine executed. Acceptable while an
instance is single-developer; a blocker if instances are ever shared. This intersects with
§15.5. See Appendix B.

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

`[OPEN: SIGN]` Code signing and notarization are entirely unspecified. An unsigned `.dmg` is
blocked by Gatekeeper, which makes the macOS build undeliverable regardless of its quality.
Subprocess spawning (§3.1) is permitted under the hardened runtime that notarization requires,
so the transport decision is compatible with direct distribution — it would not survive App
Store sandboxing. Decide alongside the client update mechanism. See Appendix B.

`[OPEN: UPDATE]` The engine self-replaces (§15.3); the client has no update mechanism. Version
skew between the two is therefore unmanaged in one direction. See Appendix B.

---

# 18. Verification and Operations

## 18.1 Network resilience testing

A mock SSH daemon simulates realistic conditions — 250 ms RTT, 5% packet loss — in CI. This
exists so the latency and resilience behaviour can be verified on every change without AWS
spend, and it is a deliverable of the first build increment, not an afterthought.

## 18.2 Test strategy

`[OPEN: TEST]` Beyond the mock daemon, testing is unspecified. Required before the first
increment is accepted: unit coverage for the framing codec and correlation registry,
integration coverage against a real `sshd` in a container, end-to-end coverage of open-edit-save
and run-a-task, and a performance gate that fails when the interaction budget regresses. See
Appendix B.

## 18.3 Observability

`[OPEN: OBS]` The intent is to monitor connection reuse efficiency, reconnection loops and time
to first interaction. Undefined: metric names, transport, retention, and the privacy position
on telemetry that would otherwise carry repository paths and file names. See Appendix B.

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

**F002 `daemon-bootstrap`** — deployment, handshake, version negotiation (§3.8). Blocked on
`[OPEN: H-BOOT]`.

**F003 `workspace-cache`** — the `WorkspaceProvider` trait (§6.1), canonical schema (§5.2),
lazy tree loading (§10.1), ranged reads, hash-based validity (§5.3). First point at which a
real repository can be browsed.

**F004 `file-watch-sync`** — engine-side watching (§10.3), `workspace/onFileEvent` emission,
the ignore set, and client-side invalidation (§10.4). Without it the client never learns that
a file changed underneath it.

**F005 `ec2-lifecycle`** — wake, stop, address resolution (§15.5). Blocked on `[OPEN: EC2]`.

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

**F014 `client-packaging`** (§17.4) — blocked on `[OPEN: SIGN]` and `[OPEN: UPDATE]`, and
nothing ships without it. **F016 `cloud-burst`** (§15.4) — requires both modes working.
**F017 `previews-artifacts`** (§3.5, §9.2, §9.3) — requires execution.

## 19.6 Not scheduled

**Debugging** (§7.4) cannot be sequenced until `[OPEN: DAP]` is closed — it has no methods, no
breakpoint model and no persistence defined, so its scope cannot be stated honestly. Note it
would also need a per-language pass equivalent to F009: delve, lldb-vscode and debugpy.

**Observability** (§18.3) likewise awaits `[OPEN: OBS]`: no metric names, transport or
retention exist to build against.

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

## A-B3 — Read-only offline (2026-09-21, provisional)

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

Practically, it also decouples the shell from F003, which sits behind `[OPEN: H-BOOT]`. Had
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

---

# Appendix B — Open Items

Blocking for the feature each appears in. Nothing here has an owner yet.

## Product decisions

| Id | Question | Consequence of leaving it open |
|---|---|---|
| **B3-reversal** | Is offline editing a product differentiator worth roughly two features? | §11 is built on a provisional engineering call (A-B3) |
| **PREVIEW** | Auto-detected previews, or user-initiated? | §9.3 unbuildable; does not block transport |
| **EC2** | Idle threshold, cost target, wake latency, provisioning, per-developer or shared instances | §15.5 unbuildable; shared instances would also reopen SEC |
| **WORKSPACE** | Remote workspace naming, collision, re-burst, deletion, quota | §15.4 has undefined behaviour on second use |

## Engineering decisions

| Id | Question | Blocks |
|---|---|---|
| **H-BOOT** | Daemon deployment, version negotiation, mismatch policy | Every remote feature; build increment 2 |
| **DAP** | Debugging protocol, breakpoint sync and persistence, variable model | Debugging entirely; absent from §19 |
| **LSP** | cgroup limits, multi-root workspaces, capability passthrough | §7.3 acceptance criteria |
| **IGNORE** | Indexing and watch exclusion set, per-workspace configuration | §10.3; affects indexer and watcher agreement |
| **D12** | Safe toolchain remediation: pinning, verification, failure path, privileges | §14.3; currently unpinned remote code execution |
| **SEC** | Authorization beyond SSH access, per-client limits, audit log | §16.3; acceptable single-tenant, blocking if shared |

## Delivery decisions

| Id | Question | Blocks |
|---|---|---|
| **SIGN** | Code signing and notarization | macOS delivery entirely — unsigned `.dmg` is blocked by Gatekeeper |
| **UPDATE** | Client update mechanism | Version skew management; pairs with H-BOOT |
| **TEST** | Test strategy beyond the mock daemon | Acceptance of every increment |
| **OBS** | Metric names, transport, retention, telemetry privacy | §18.3 |
| **NFR** | Percentiles, measurement points, exclusions for the §1.4 targets | Any performance gate |

## Provenance

The source of this specification was an LLM-authored architecture narrative. It carried
generation scaffolding into several sections, named a Rust crate that does not exist, included
a sample that did not compile, and specified two different schemas and two different protocol
contracts. All of that has been corrected or removed.

The working assumption for anyone extending this document: named crates, flags and figures are
claims to verify, not decisions already validated.
