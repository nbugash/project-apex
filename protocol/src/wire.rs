//! The messages the client and engine exchange, from §4.8.
//!
//! Shared rather than duplicated: a field name that differs between the two ends fails at the
//! far end, where the evidence is worst.

use serde::{Deserialize, Serialize};

/// What this build of the protocol speaks.
///
/// Incremented on a **breaking** change only — never for an added method, an added optional
/// parameter or an added result field. A version that increments on additions forces a
/// redeployment across every host for changes that needed none, and it is only safe because
/// both ends are required to ignore what they do not recognise.
pub const PROTOCOL_VERSION: u32 = 1;

/// An opaque session identity, minted by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What one side can do: opaque tokens, compared by exact match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet(pub Vec<String>);

impl CapabilitySet {
    pub fn of(tokens: &[&str]) -> Self {
        Self(tokens.iter().map(|t| t.to_string()).collect())
    }

    /// Exact match, never a prefix. A prefix test would make `workspace/read` satisfy a
    /// requirement for `workspace/readFile`.
    pub fn has(&self, token: &str) -> bool {
        self.0.iter().any(|t| t == token)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeRequest {
    pub client_version: String,
    pub protocol_version: u32,
    pub capabilities: CapabilitySet,
    /// Present when re-attaching after a disconnection; absent on a first connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_session: Option<SessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeResponse {
    pub engine_version: String,
    pub protocol_version: u32,
    pub capabilities: CapabilitySet,
    pub session_id: SessionId,
    /// False means a new session was created. The client must surface that rather than treat
    /// it as success — a client that silently continues shows work that is not happening.
    pub resumed: bool,
}

/// Sent by the engine after it re-executes itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartNotice {
    pub session_id: SessionId,
    /// Everything that did not survive. Empty is a positive assertion that nothing was lost,
    /// not an absence of information.
    pub unpreserved: Vec<String>,
}

/// Application error codes from §4.4. Both ends must agree, and neither may write the integer
/// inline — a literal `-32001` in a match arm is a fact stated twice.
pub mod codes {
    /// The engine has never been told about this workspace. The client's answer is to register
    /// it, which is also how it recovers after an engine restart.
    pub const WORKSPACE_NOT_REGISTERED: i32 = -32001;
    /// Path escapes the workspace root (§4.7). Returned identically whether or not the escaped
    /// target exists, so a refusal cannot be used to probe the host's filesystem.
    pub const PATH_REFUSED: i32 = -32002;
    /// Inside the root, and absent. Information the caller is entitled to.
    pub const NOT_FOUND: i32 = -32003;
    /// `baseSha256` does not match the file's current content, so the write was refused and
    /// **nothing was written**. §4.4 specified this from the start and nothing defined it until
    /// F006, because nothing could write.
    ///
    /// It means the base mismatch and only that. A missing file, an unreadable one and a
    /// permission failure each have their own code, and collapsing any of them into this would
    /// make a client's "somebody else edited this" message a lie in three other situations.
    pub const WRITE_CONFLICT: i32 = -32004;
    /// Frame exceeds §4.1's cap.
    pub const PAYLOAD_TOO_LARGE: i32 = -32007;
    /// The engine is re-executing. §4.8's `session/restart` prose assigns this meaning; it had
    /// no row in §4.4's table and no constant here, so `rpc.rs` wrote the integer inline in
    /// defiance of this module's own rule.
    pub const RESTARTING: i32 = -32000;
    /// Registered, but the root directory is gone. **Not** `WORKSPACE_NOT_REGISTERED`: that one
    /// means re-register, and re-registering a deleted root fails on the not-a-directory check,
    /// surfacing a registration error for a deletion.
    pub const WORKSPACE_GONE: i32 = -32009;
    /// The identity is unknown, and **not** that the task has finished. §4.4 read "Task not found
    /// or already exited" until F010, which contradicted two requirements at once: stopping a
    /// stopped task is a success, since the caller asked for it not to be running and it is not
    /// running, and a client reattaching to a task that ended while it was away is entitled to
    /// learn how it ended rather than be told the task never existed.
    pub const TASK_NOT_FOUND: i32 = -32006;
    /// The identity is live. The client's answer is to **attach**, not to retry, which is why
    /// this cannot be `TASK_NOT_FOUND` -- that one means the opposite. Without a code of its own
    /// a client racing its own reconnection could not tell "your build is already running" from
    /// "there is no such build" (FR-031c, SC-022).
    pub const TASK_ALREADY_RUNNING: i32 = -32010;
    /// The command itself could not be started: not found, not executable, or `cwd` unusable.
    /// The developer's mistake to fix rather than the engine's failure. **Not** `NOT_FOUND`,
    /// which is reserved for paths inside a workspace -- a program name resolved against `PATH`
    /// is not a workspace path at all (FR-004, SC-015).
    pub const COMMAND_NOT_STARTED: i32 = -32011;
}

/// An opaque workspace identity, minted by the client (A-WORKSPACE).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub String);

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What a directory entry is. Serialised lowercase so the wire reads as §4.8 writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Directory,
}

/// One child of a directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FsEntryWire {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: EntryKind,
    pub size: u64,
    /// Unix seconds.
    pub modified: i64,
}

// ---- workspace/register ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterParams {
    pub workspace_id: WorkspaceId,
    /// Absolute, on the engine's host. Untrusted, like every path off the wire.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResult {
    pub name: String,
    pub canonical_path: String,
}

// ---- workspace/writeFile ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteFileParams {
    pub workspace_id: WorkspaceId,
    /// Workspace-relative. Untrusted, like every path off the wire (§4.7).
    pub relative_path: String,
    /// The whole file. §4.8 carries content, not a patch, so there is no partial write to
    /// express and a client holding part of a file has nothing safe to send.
    pub content: String,
    /// The hash the client believed current when it began editing. The engine compares this
    /// with what is on disk and refuses a mismatch rather than overwriting.
    pub base_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteFileResult {
    /// The hash of what was written, computed after writing rather than taken from the request.
    /// A client adopting it as its new base is adopting what is on disk.
    pub sha256: String,
}

// ---- workspace/readDirectory ----

/// The largest page §4.8 permits, and the default when a caller states none.
///
/// Not a preference: a listing of a hundred thousand entries at roughly a hundred bytes each is
/// an order of magnitude past §4.1's 1 MiB cap, so an unpaged listing is undeliverable rather
/// than merely slow.
pub const MAX_DIRECTORY_PAGE: u32 = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadDirectoryParams {
    pub workspace_id: WorkspaceId,
    pub relative_path: String,
    /// The last `name` of the previous page. Stateless: the engine keeps no iterator, so a page
    /// may be requested at any time, in any order, after any restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Clamped to `MAX_DIRECTORY_PAGE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadDirectoryResult {
    /// Ordered `(type DESC, name ASC)` — directories first, then by name, byte-wise on UTF-8.
    /// **The ordering is contractual**, because the cursor is the last name returned.
    pub items: Vec<FsEntryWire>,
    /// Present exactly when more entries follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---- workspace/stat ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatParams {
    pub workspace_id: WorkspaceId,
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatResult {
    #[serde(rename = "type")]
    pub kind: EntryKind,
    pub size: u64,
    pub modified: i64,
    /// Lowercase hex of the file's whole content. Absent for a directory: there is nothing to
    /// hash and no caller that needs it. This is the sole input to cache validity (§5.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

// ---- workspace/readFile ----

/// The largest raw payload one frame may carry (A-BULKSIZE).
///
/// Below §4.1's 1 MiB cap by design, not by caution: content is base64 on the wire, four bytes
/// out for every three in, so 512 KiB raw becomes roughly 683 KiB encoded and leaves room for the
/// envelope. A threshold at the cap would encode past it and fail on the first frame with a path.
pub const MAX_INLINE_READ: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileParams {
    pub workspace_id: WorkspaceId,
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileResult {
    /// Base64. There is no utf8 path: assuming text corrupts binary content silently, and a
    /// method that sometimes returns text makes every caller branch on it.
    pub content: String,
    pub encoding: String,
    /// The **whole file's** hash, never the returned range's. A caller assembling ranges compares
    /// this across them; a change means the file moved underneath the read.
    pub sha256: String,
    pub total_size: u64,
}

// ---- workspace/watch, workspace/unwatch ----

/// Paths the client cares about: folder paths for expanded folders, **file** paths for open
/// tabs. Not the directories the engine will watch -- it derives those, because a folder
/// holding an open file arrives as one path for two reasons and unwatching on a collapse must
/// not be mistaken for unwatching on a tab close (A-WATCHSCOPE, FR-003c).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchParams {
    pub workspace_id: WorkspaceId,
    /// Untrusted, like every path off the wire.
    pub paths: Vec<String>,
}

/// Why one path is not being watched. A closed set of four; a client reads an unrecognised
/// value as `Capacity`, the conservative reading, because every reason means the same thing to
/// the developer: this path is not being watched (FR-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// The host cannot hold another watch. Not an error: FR-005a keeps the workspace usable.
    Capacity,
    /// Inside the exclusion set (FR-008). Reported rather than silently skipped, because
    /// silence is indistinguishable from a working watch.
    Excluded,
    NotFound,
    NotADirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchRefusal {
    pub path: String,
    pub reason: RefusalReason,
}

/// A refusal is data, not an error. The call succeeds and reports what it could not do, which
/// is what lets an exhausted host still open and browse a workspace (FR-005, FR-005a).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchResult {
    /// The size of the requested set after the call. **Not** the number of host watch
    /// descriptors: the two differ by ancestors and the root, and the criteria that measure
    /// descriptors read them through the port instead.
    pub watching: u32,
    pub refused: Vec<WatchRefusal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnwatchResult {
    pub watching: u32,
}

// ---- workspace/onFileEvent ----

/// What happened to one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileEventKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// One change. Carries no content -- no bytes and no hash (FR-013) -- and so cannot make a
/// client believe it holds current bytes. It does carry the entry metadata a listing already
/// returns (FR-013a), without which a created file could not be placed in the tree at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEvent {
    pub event: FileEventKind,
    pub relative_path: String,
    /// Present exactly when `event` is `Renamed`. That is FR-011 expressed as a shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_path: Option<String>,
    /// On `Created` and `Modified`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<EntryKind>,
    /// On `Created` and `Modified`, for a file. A directory has nothing to measure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Unix **seconds**, the unit `FsEntryWire` already uses. The justification for carrying
    /// this at all is that it is the same metadata a listing returns, which it would stop
    /// being if it arrived in a different unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
}

/// One flush of the coalescer is one frame. §4.6 makes this one pipe and one queue, and a
/// burst delivered as hundreds of frames would take the writer hundreds of times ahead of
/// whatever interactive request is behind it (FR-016, A-COALESCE).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEventParams {
    pub workspace_id: WorkspaceId,
    /// Never empty.
    pub events: Vec<FileEvent>,
}

// ---- workspace/invalidateAll ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidateAllParams {
    pub workspace_id: WorkspaceId,
}

// ---- execution/* (F010) ----

/// A task's identity, chosen by the client (§4.8).
///
/// **Unique across the engine, not within a workspace.** Six of §4.8's nine execution rows carry
/// a bare `task_id`, so a per-workspace identity would leave them unable to resolve a task at
/// all. The `workspace_id` on `runTask` and `attach` records which workspace owns the task, not
/// which namespace its name lives in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A process id.
///
/// A newtype rather than a bare `i32` so it cannot be passed where an exit code or a signal
/// number is wanted. All three are small integers and all three appear together on `onExit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pid(pub i32);

/// A signal's **name**, as it appears on the wire.
///
/// Names and not numbers, because signal numbers differ between platforms and the client is not
/// always on the engine's. A client composing a stop should not have to know Linux's numbering,
/// and an unrecognised name can be refused where an unrecognised number cannot be told from a
/// valid one.
///
/// This is the **reporting** vocabulary and it is deliberately open: a task may be killed by
/// anything the kernel can deliver -- `SIGSEGV` from its own bug, `SIGPIPE`, `SIGKILL` from the
/// out-of-memory killer -- and FR-021 requires reporting the signal that killed it, not the
/// signal someone asked for. Contrast `TerminateSignal`, which is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalName(pub String);

impl SignalName {
    /// Total over the host's signals, including ones it has no name for.
    ///
    /// An unrecognised number formats as `SIG<n>` rather than being dropped or replaced with a
    /// placeholder: a signal nobody anticipated is still how the task died, and FR-021's "in
    /// 100% of exercised cases" does not exempt the unfamiliar.
    pub fn from_number(sig: i32) -> Self {
        let name = match sig {
            1 => "SIGHUP",
            2 => "SIGINT",
            3 => "SIGQUIT",
            4 => "SIGILL",
            6 => "SIGABRT",
            8 => "SIGFPE",
            9 => "SIGKILL",
            11 => "SIGSEGV",
            13 => "SIGPIPE",
            14 => "SIGALRM",
            15 => "SIGTERM",
            24 => "SIGXCPU",
            25 => "SIGXFSZ",
            28 => "SIGWINCH",
            _ => return SignalName(format!("SIG{sig}")),
        };
        SignalName(name.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a client may ask `execution/terminate` to send.
///
/// Closed at three, which is the **sending** vocabulary. Whether it escalates follows from which
/// one it is: `Term` escalates to `Kill` after the grace period, because a stop a process can
/// decline is not a stop; `Int` does not, because a program handling it as designed -- a test
/// runner printing a summary, a shell returning to its prompt -- must not be killed for having
/// handled it.
///
/// Per-variant renames and not `rename_all = "UPPERCASE"`: that would emit `"INT"`, not
/// `"SIGINT"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminateSignal {
    #[serde(rename = "SIGINT")]
    Int,
    #[serde(rename = "SIGTERM")]
    Term,
    #[serde(rename = "SIGKILL")]
    Kill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTaskParams {
    pub workspace_id: WorkspaceId,
    pub task_id: TaskId,
    /// An **argv vector**, never a shell line. The engine interposes no `sh -c`: §7.3 scopes
    /// this as process execution and not a shell, and a single string would make quoting the
    /// engine's problem for input it is specifically required not to interpret.
    pub command: Vec<String>,
    /// Absent is the workspace root, which is where a build usually runs. Untrusted, like every
    /// path off the wire (§4.7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Absent inherits the engine's environment unchanged; present is merged **over** that
    /// inheritance rather than replacing it. Replacement is the more obvious reading of a bare
    /// parameter and the wrong default: a task started with one variable set would lose `PATH`
    /// and fail for a reason that looks nothing like its cause.
    ///
    /// `BTreeMap` so serialisation is deterministic; a test asserting on a frame should not
    /// depend on hash order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::BTreeMap<String, String>>,
    /// Chooses between two output shapes, and the choice is exclusive because a terminal is one
    /// device (A-TASKSTREAM). With `true` the streams arrive merged on `onStdout` and `onStderr`
    /// carries nothing.
    pub pty: bool,
    /// Meaningful only when `pty` is true. Absent defaults to 80 x 24, and specifically not the
    /// kernel's 0 x 0 -- a size no display has, and the one value `resizePty` refuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTaskResult {
    pub pid: Pid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachParams {
    pub workspace_id: WorkspaceId,
    pub task_id: TaskId,
}

/// The state of a task a client has just reached.
///
/// `retained` is a **byte count**, not the bytes: the retention bound is larger than §4.1's frame
/// cap, chunking is defined for notifications rather than results, and an exit delivered inside
/// the result would arrive before the output that preceded it. The bytes are replayed as ordinary
/// `onStdout` and `onStderr` notifications after the response, each chunk on the notification its
/// own stream would have used when live.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachResult {
    pub pid: Pid,
    pub running: bool,
    pub retained: u64,
    /// Exactly one of `exit_code` and `signal` is present once `running` is false, and neither
    /// while it is true. `running: false` alone says only that it is over; FR-031b and SC-020
    /// require a reattaching client to learn **how** it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<SignalName>,
}

/// Omitted `workspace_id` lists every task the engine holds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<WorkspaceId>,
}

/// One entry of a listing.
///
/// Deliberately carries **no `env`**: FR-005a keeps a task's environment out of anything that can
/// be read back, and a listing is exactly that. It does carry `command`, which moves where
/// FR-005a's accepted boundary sits -- a credential in argv becomes readable by any client that
/// enumerates, not only the one that started the task. Under A-EC2's single tenancy that is the
/// same developer, so it widens *where* rather than *who*.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub command: Vec<String>,
    pub pty: bool,
    pub pid: Pid,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<SignalName>,
}

/// Unpaged, unlike `workspace/readDirectory`. Bounded only by how many tasks one developer has
/// started; the arithmetic that could exceed §4.1's cap is accepted and recorded in §4.8.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResult {
    pub tasks: Vec<TaskSummary>,
}

/// `workspace/register`'s counterpart. Deliberately **not** the same event as a dropped
/// connection: under A-TASKLIFE a drop leaves tasks running, because a laptop moving between
/// networks must not kill a build, whereas closing the workspace is the developer saying they are
/// done with it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCloseParams {
    pub workspace_id: WorkspaceId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteStdinParams {
    pub task_id: TaskId,
    /// Base64. A JSON string holds Unicode text and a task's input is not text -- a paste may
    /// carry any byte, and a lossy decode substitutes U+FFFD and destroys what it cannot read.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResizePtyParams {
    pub task_id: TaskId,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminateParams {
    pub task_id: TaskId,
    /// The **initial** signal. See `TerminateSignal` for what escalates and what does not.
    pub signal: TerminateSignal,
}

/// One chunk of a task's output, on `execution/onStdout` or `execution/onStderr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputParams {
    pub task_id: TaskId,
    /// Base64, for the reason `WriteStdinParams::data` is: a compiler emitting bytes in the
    /// source file's own encoding, a binary written to stdout and a file catted to a terminal
    /// are all ordinary, and none survives a lossy decode (FR-009, SC-003).
    pub data: String,
}

/// How a task ended.
///
/// Exactly one of `exit_code` and `signal`, never both and never neither. A mandatory
/// `exit_code` would leave a signalled death representable only through the `128 + n` convention,
/// which is what a shell does for a human reading a number and not what a protocol should require
/// a client to decode. A client reads `signal` first: its presence means the task was signalled,
/// whatever else is on the frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitParams {
    pub task_id: TaskId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<SignalName>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_match_exactly_and_never_by_prefix() {
        let c = CapabilitySet::of(&["workspace/readFile", "session/shutdown"]);
        assert!(c.has("workspace/readFile"));
        assert!(
            !c.has("workspace/read"),
            "a prefix must not satisfy a capability check"
        );
        assert!(!c.has("workspace/readFileExtra"));
        assert!(!c.has(""));
    }

    #[test]
    fn an_absent_resume_session_is_omitted_from_the_wire() {
        let r = HandshakeRequest {
            client_version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: CapabilitySet::default(),
            resume_session: None,
        };
        let json = serde_json::to_string(&r).expect("serialise");
        assert!(!json.contains("resume_session"), "{json}");
    }
}
