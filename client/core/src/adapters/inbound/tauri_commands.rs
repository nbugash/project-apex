//! The `ShellCommands` inbound adapter.
//!
//! Input arriving here is untrusted regardless of interface-layer validation (Constitution
//! Principle VI). Every argument is validated in the core, which rejects rather than coerces.

use crate::adapters::inbound::task_commands::Tasks;
use crate::application::error::ShellError;
use crate::application::ports::workspace_cache::WorkspaceCache;
use crate::application::use_cases::edit_file::{EditFile, WriteOutcome};
use crate::application::use_cases::observe_connection::ObserveConnection;
use crate::application::use_cases::persist_session::PersistSession;
use crate::application::use_cases::retain_edit::RetainEdit;
use crate::domain::connection::ConnectionState;
use crate::domain::layout::RegionId;
use crate::domain::rail::{DestinationId, RailCatalogue, ToolWindowState};
use crate::domain::session::{DocumentId, SessionSnapshot};
use crate::window::controller::WindowController;
use std::sync::Arc;
use tauri::{Emitter, State};

pub struct Shell {
    pub persist: Arc<PersistSession>,
    pub connection: Arc<ObserveConnection>,
    pub window: Arc<WindowController>,
    pub rail: Arc<RailCatalogue>,
    /// Debug builds only: lets the end-to-end suite drive connection transitions. Absent
    /// from release builds, so it cannot become a production surface by accident.
    #[cfg(debug_assertions)]
    pub stub: Arc<crate::adapters::outbound::stub_connection::StubConnectionStatusSource>,
}

/// Region identifiers arrive as strings from the bridge and are not trusted to be valid.
fn parse_region(raw: &str) -> Result<RegionId, ShellError> {
    match raw {
        "output" => Ok(RegionId::Output),
        "document_area" => Ok(RegionId::DocumentArea),
        _ => Err(ShellError::InvalidRegion),
    }
}

#[tauri::command]
pub fn shell_ready(shell: State<'_, Shell>) -> Result<(), ShellError> {
    shell.window.mark_ready().map_err(|e| {
        crate::logging::warn(&format!("show failed: {e}"));
        ShellError::PersistenceFailed
    })
}

/// The restored session, and the workspace identity that comes with it.
///
/// **The restore also tells the core which workspace is current.** `workspace_open` sets that,
/// and a relaunch does not call it -- the session restores the workspace instead. Without this
/// the core came back up not knowing which workspace it had, so every command that resolves the
/// workspace itself (Principle VI) answered `UnknownWorkspace` until the developer opened one
/// by hand. It showed as git marks that survived a restart in the database and not on screen.
///
/// The engine outlives the client (A-ENGINELIFE), so its own registration is still there; what
/// was missing was only this side's memory of which one it was.
#[tauri::command]
pub fn session_get(shell: State<'_, Shell>, tasks: State<'_, Tasks>) -> SessionSnapshot {
    let snapshot = shell.persist.snapshot();
    if let Some(ws) = snapshot.workspace.as_ref() {
        let mut current = tasks.current.lock().expect("current workspace");
        // Only when nothing has been opened since. A restore must never displace a workspace
        // the developer opened in this session.
        if current.is_none() {
            *current = Some(ws.id.clone());
        }
    }
    snapshot
}

#[tauri::command]
pub fn layout_set_region(
    region: String,
    visible: bool,
    extent: u32,
    shell: State<'_, Shell>,
) -> Result<(), ShellError> {
    shell
        .persist
        .set_region(parse_region(&region)?, visible, extent)
}

/// Turn autosave on or off (FR-007b, FR-042).
#[tauri::command]
pub fn session_set_autosave(on: bool, shell: State<'_, Shell>) -> Result<(), ShellError> {
    shell.persist.set_autosave(on)
}

#[tauri::command]
pub fn documents_open(
    display_name: String,
    path: String,
    shell: State<'_, Shell>,
) -> Result<DocumentId, ShellError> {
    shell.persist.open_document(&display_name, &path)
}

#[tauri::command]
pub fn documents_close(id: String, shell: State<'_, Shell>) -> Result<(), ShellError> {
    shell.persist.close_document(&DocumentId(id))
}

#[tauri::command]
pub fn documents_reorder(
    id: String,
    to_order: u32,
    shell: State<'_, Shell>,
) -> Result<(), ShellError> {
    shell.persist.reorder_document(&DocumentId(id), to_order)
}

#[tauri::command]
pub fn documents_focus(id: String, shell: State<'_, Shell>) -> Result<(), ShellError> {
    shell.persist.focus_document(&DocumentId(id))
}

/// Debug-only test hook. See `Shell::stub`.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn stub_set_connection(state: String, shell: State<'_, Shell>) -> Result<(), ShellError> {
    use crate::domain::connection::ConnectionState::*;
    let next = match state.as_str() {
        "unknown" => Unknown,
        "connecting" => Connecting,
        "connected" => Connected,
        "disconnected" => Disconnected,
        _ => return Err(ShellError::InvalidRegion),
    };
    shell.stub.set(next);
    Ok(())
}

/// Destination identifiers arrive as strings and are matched against the catalogue rather
/// than trusted — the same rule as region identifiers above.
#[tauri::command]
pub fn rail_select(
    destination_id: String,
    shell: State<'_, Shell>,
) -> Result<ToolWindowState, ShellError> {
    shell
        .persist
        .select_destination(&DestinationId(destination_id), &shell.rail)
}

#[tauri::command]
pub fn tool_window_resize(width: u32, shell: State<'_, Shell>) -> Result<(), ShellError> {
    shell.persist.resize_tool_window(width)
}

#[tauri::command]
pub fn rail_destinations(shell: State<'_, Shell>) -> Vec<RailDestinationView> {
    shell
        .rail
        .all()
        .iter()
        .map(|d| RailDestinationView {
            id: d.id.0.clone(),
            label: d.label.to_string(),
            icon: d.icon.to_string(),
            available: d.is_selectable(),
            order: d.order,
        })
        .collect()
}

/// The bridge shape for a destination. Separate from the domain type because `&'static str`
/// labels do not cross a serialisation boundary, and the interface has no use for the
/// availability enum's spelling.
#[derive(serde::Serialize)]
pub struct RailDestinationView {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub available: bool,
    pub order: u32,
}

#[tauri::command]
pub fn connection_current(shell: State<'_, Shell>) -> crate::domain::connection::ConnectionState {
    shell.connection.current()
}

// ---- The editor's file surface (F006) ----

/// The largest file this editor opens as text (plan.md, *Fixed Quantities*).
///
/// Monaco holds the whole model in memory once loaded, so past this the window stops responding
/// rather than merely being slow. A refusal naming the limit is worse than opening the file and
/// far better than a hang the developer cannot escape.
pub const MAX_TEXT_FILE: u64 = 64 * 1024 * 1024;

/// A piece of a file, as text.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkDto {
    pub text: String,
    /// Of the **whole** file, never of `text`. It is what a save is conditional on, and what a
    /// caller assembling several ranges compares across them.
    pub sha256: String,
    /// The whole file's size, so a partial read knows what it is part of.
    pub total: u64,
    /// Where `text` begins. Not always what was asked for: a range boundary can land inside a
    /// multi-byte character, and the answer is trimmed to whole characters.
    pub offset: u64,
}

/// What a save produced, as five cases the interface switches on.
///
/// Carried in the success channel deliberately. Three of these are failures, but they are
/// failures the interface must *branch* on rather than merely report, and splitting them across
/// `Ok` and `Err` would push the caller back to inspecting an error to find out which it was --
/// which is what the typed variants exist to prevent.
///
/// `HeldLocally` is F012's, and it is a **success**: the work is on this machine and will reconcile
/// when the connection returns. It sits here for the same reason as the rest -- the interface must
/// branch on it -- and putting it in `Err` would have made a retained save arrive through the
/// failure path, which is the confusion §11.2 forbids.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WriteOutcomeDto {
    Written { sha256: String },
    HeldLocally,
    Conflict,
    Refused { message: String },
    Unreachable,
}

impl From<WriteOutcome> for WriteOutcomeDto {
    fn from(o: WriteOutcome) -> Self {
        match o {
            WriteOutcome::Written { sha256 } => Self::Written {
                sha256: sha256.to_string(),
            },
            WriteOutcome::Conflict => Self::Conflict,
            WriteOutcome::Refused { reason } => Self::Refused { message: reason },
            WriteOutcome::Unreachable => Self::Unreachable,
        }
    }
}

/// The workspace a file command acts in.
///
/// Read from the core rather than accepted from the webview, the same Principle VI decision
/// F010 made for tasks: a `workspaceId` supplied by the interface is the interface choosing
/// which workspace a command reaches, and it has no business choosing that. The core registered
/// the workspace, so the core knows which one it is.
fn current_workspace(tasks: &Tasks) -> Result<WorkspaceId, WorkspaceFailure> {
    tasks
        .current
        .lock()
        .expect("current workspace")
        .clone()
        .map(WorkspaceId)
        .ok_or(WorkspaceFailure::UnknownWorkspace)
}

/// Untrusted input, refused rather than repaired into something that parses.
fn editor_path(raw: &str) -> Result<RelPath, WorkspaceFailure> {
    RelPath::parse(raw).map_err(|_| WorkspaceFailure::Refused)
}

/// Decode a chunk's bytes as text, trimming a range's edges to whole characters.
///
/// Trimming is not lossy: the bytes dropped belong to a character the adjacent range delivers
/// whole. A lossy decode would be -- it replaces the partial character with U+FFFD, and the
/// developer saves that substitution back over their file. `Utf8Error::error_len` separates the
/// two cases exactly: `None` means the input ended mid-character, `Some` means a sequence that
/// is invalid wherever it appears, which is binary content and belongs to F017 (FR-006).
fn decode_chunk(chunk: FileChunk) -> Result<ChunkDto, WorkspaceFailure> {
    if chunk.total_size > MAX_TEXT_FILE {
        return Err(WorkspaceFailure::TooLarge(chunk.total_size));
    }

    let bytes = &chunk.bytes[..];
    // Leading continuation bytes can only be the tail of a character the previous range holds.
    // Only when the range starts partway in: at offset zero they are invalid, not partial.
    let mut start = 0;
    if chunk.range.offset > 0 {
        while start < bytes.len() && (bytes[start] & 0xC0) == 0x80 {
            start += 1;
        }
    }
    let rest = &bytes[start..];

    let end = match std::str::from_utf8(rest) {
        Ok(_) => rest.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => return Err(WorkspaceFailure::NotText),
    };

    let text = std::str::from_utf8(&rest[..end])
        .map_err(|_| WorkspaceFailure::NotText)?
        .to_string();

    Ok(ChunkDto {
        text,
        sha256: chunk.sha256.to_string(),
        total: chunk.total_size,
        offset: chunk.range.offset + start as u64,
    })
}

/// Read a whole file as text.
///
/// Unranged on purpose: only an unranged read is cached (`CachedWorkspace` stores whole content
/// and nothing else), so asking for a range here would make every reopen cost a request and
/// SC-002 unachievable.
#[tauri::command]
pub async fn file_read(
    path: String,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<ChunkDto, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    let rel = editor_path(&path)?;

    // A path with retained work reads as the developer's content (FR-013). See `pending_chunk`.
    if let Some(chunk) = pending_chunk(access.cache.as_ref(), &ws, &rel)? {
        return Ok(chunk);
    }

    match access.provider.read_file(&ws, &rel, None).await {
        Ok(chunk) => decode_chunk(chunk),
        Err(ProviderError::TooLarge { .. }) => {
            // The refusal says the file is above the inline limit but not always how far: §4.8
            // gives it no structured field for the size. So ask the method whose job is
            // reporting size. One extra round trip, and only for a file that is about to cost
            // many more, against the alternative of reading a number out of a sentence.
            let meta = access.provider.stat(&ws, &rel).await?;
            Err(WorkspaceFailure::TooLarge(meta.size))
        }
        Err(e) => Err(e.into()),
    }
}

/// Read one window of a file (FR-017).
///
/// Deliberately not cached: the projection holds whole files, and storing a fragment under a
/// whole file's digest would make the next validity check agree with content that is not there.
#[tauri::command]
pub async fn file_read_range(
    path: String,
    offset: u64,
    len: u64,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<ChunkDto, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    let rel = editor_path(&path)?;
    let chunk = access
        .provider
        .read_file(
            &ws,
            &rel,
            Some(ByteRange {
                offset,
                length: len,
            }),
        )
        .await?;
    decode_chunk(chunk)
}

/// The file's current digest, or `None` when it has none.
///
/// Exists for A-WRITEECHO: deciding whether a file event describes our own write or somebody
/// else's change is a hash comparison, and nothing else can answer it.
#[tauri::command]
pub async fn file_hash(
    path: String,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<Option<String>, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    let rel = editor_path(&path)?;
    let meta = access.provider.stat(&ws, &rel).await?;
    Ok(meta.sha256.map(|h| h.to_string()))
}

/// Save, conditional on the base the buffer held (FR-007).
#[tauri::command]
pub async fn file_write(
    path: String,
    content: String,
    base: String,
    shell: State<'_, Shell>,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<WriteOutcomeDto, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    let rel = editor_path(&path)?;
    // A base that is not a digest cannot have come from a read this client performed. Refused
    // rather than forwarded: the engine would compare it, fail to match and answer `-32004`,
    // and the developer would be told a colleague edited their file when nobody did.
    let base = Sha256::parse(&base).ok_or(WorkspaceFailure::Refused)?;

    // Offline, the save is held rather than attempted (FR-010, FR-011).
    //
    // Routed here rather than inside `CachedWorkspace::write_file`, which already checks the
    // connection: that would widen the caching layer's job and hide the offline branch from where
    // the `HeldLocally` outcome is produced. `RetainEdit` is constructed from `access.cache`, which
    // this struct already carries so a command can write through the same port the application
    // reads through.
    if !matches!(shell.connection.current(), ConnectionState::Connected) {
        // The base is the content the host last confirmed, which is what the cache holds. `None`
        // when there is no cached copy: a file created offline has nothing to differ from, and a
        // cached copy that has since been evicted is why the base is *stored* with the edit rather
        // than referenced from the cache (A-PENDING).
        let held = access
            .cache
            .lookup(&ws, &rel)
            .ok()
            .flatten()
            .map(|entry| (entry.bytes, entry.hash));
        let base_pair = held.as_ref().map(|(bytes, hash)| (bytes.as_slice(), hash));
        // Mergeable is decided by what the client holds, never by sniffing: the content arrived
        // here as a `String`, so it is text by construction. A file the editor could not decode
        // never reaches this command with content to save.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        return match RetainEdit::new(access.cache.clone()).save(
            &ws,
            &rel,
            content.as_bytes(),
            base_pair,
            true,
            now,
        ) {
            Ok(()) => Ok(WriteOutcomeDto::HeldLocally),
            // FR-016: the developer is told while the work is still in the buffer. A store that
            // refused is not a write conflict and not an unreachable host, so it is `Refused` with
            // the reason -- the one outcome that says "this will not work until something changes".
            Err(e) => Ok(WriteOutcomeDto::Refused {
                message: format!("the edit could not be held locally: {e}"),
            }),
        };
    }

    let outcome = EditFile::new(access.provider.clone())
        .save(&ws, &rel, &content, &base)
        .await;
    Ok(outcome.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_region_identifiers_are_rejected_at_the_boundary() {
        assert_eq!(parse_region("../../etc"), Err(ShellError::InvalidRegion));
        assert_eq!(parse_region(""), Err(ShellError::InvalidRegion));
        assert_eq!(parse_region("Navigation"), Err(ShellError::InvalidRegion));
    }

    #[test]
    fn unknown_destination_identifiers_are_rejected_at_the_boundary() {
        let catalogue = RailCatalogue::default();
        let mut state = ToolWindowState::default();
        assert!(state
            .select(&DestinationId::new("../../etc"), &catalogue)
            .is_err());
        assert!(state.select(&DestinationId::new(""), &catalogue).is_err());
    }

    #[test]
    fn known_region_identifiers_parse() {
        // "navigation" is no longer a region: F018 replaced it with the tool window,
        // which carries its own state rather than being a generic resizable slot.
        assert_eq!(parse_region("navigation"), Err(ShellError::InvalidRegion));
        assert_eq!(parse_region("output"), Ok(RegionId::Output));
        assert_eq!(parse_region("document_area"), Ok(RegionId::DocumentArea));
    }
}

// ---------------------------------------------------------------------------
// Workspace (F003).
//
// Every argument arriving here is untrusted regardless of what the interface layer did with it
// (Principle VI). `RelPath::parse` rejects rather than coerces, and the engine validates again
// on its own side — this check protects against bugs in our own interface, never against a
// stale or hostile caller.

use crate::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use crate::domain::workspace::{ByteRange, FileChunk, PageRequest, RelPath, Sha256, WorkspaceId};
use serde::Serialize;

/// One directory entry, as the interface sees it.
#[derive(Debug, Clone, Serialize)]
pub struct EntryDto {
    pub name: String,
    /// `"file"` or `"directory"`, matching the wire vocabulary so one word means one thing
    /// everywhere.
    #[serde(rename = "kind")]
    pub kind: String,
    pub size: u64,
    pub modified: i64,
}

/// What the interface is told when a workspace request fails.
///
/// The variants a caller must distinguish, not a message it has to parse. `Gone` is separate
/// from `Offline` because they lead to opposite responses: an outage is temporary and the
/// projection is still true, while a deleted workspace means the thing being projected does not
/// exist (FR-038).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum WorkspaceFailure {
    NotFound,
    Refused,
    UnknownWorkspace,
    Gone,
    Offline,
    Unsupported(String),
    Transport(String),
    /// The file changed on the host since it was read (`-32004`). Its own variant because the
    /// interface has to tell a colleague's edit from a dropped link (FR-012).
    Conflict,
    /// The content is not text, so this editor will not present it (FR-006).
    ///
    /// Separate from `Refused` because the remedy is different and nothing about the path is
    /// wrong: the file is fine, this surface is the wrong one for it, and F017 is the one that
    /// will render it.
    NotText,
    /// Larger than this surface will open, carrying the size so the interface can say how much.
    ///
    /// A number rather than a sentence, because "too large" without the size tells a developer
    /// nothing they can act on.
    TooLarge(u64),
}

impl From<ProviderError> for WorkspaceFailure {
    fn from(e: ProviderError) -> Self {
        match e {
            ProviderError::NotFound => Self::NotFound,
            ProviderError::Refused => Self::Refused,
            ProviderError::UnknownWorkspace => Self::UnknownWorkspace,
            ProviderError::WorkspaceGone => Self::Gone,
            ProviderError::Offline => Self::Offline,
            ProviderError::TooLarge { total_size } => {
                Self::Transport(format!("{total_size} bytes exceeds the inline read limit"))
            }
            ProviderError::Transport(why) => Self::Transport(why),
            // The three task refusals reach the interface as transport-level prose, because
            // `WorkspaceFailure` is the *workspace* surface and none of them is about a
            // workspace. F010's panel takes `ProviderError` directly and branches on the
            // variants; flattening them here would be the interface losing a distinction the
            // wire spent three codes preserving, which is why each says which one it was.
            task @ (ProviderError::TaskNotFound
            | ProviderError::TaskAlreadyRunning
            | ProviderError::CommandNotStarted { .. }) => Self::Transport(task.to_string()),
            ProviderError::WriteConflict => Self::Conflict,
            ProviderError::Unsupported { owner } => Self::Unsupported(owner.to_string()),
        }
    }
}

/// The workspace surface the interface reaches.
///
/// Held separately from `Shell` because it exists only once a workspace has been opened, and a
/// field that is always `None` before then would put an unwrap in every command.
pub struct WorkspaceAccess {
    pub provider: Arc<dyn WorkspaceProvider>,
    /// How git status is asked for and applied. `None` when no engine is configured, which is
    /// the same condition under which there is nothing to ask.
    pub git: Option<Arc<crate::application::use_cases::apply_git_status::ApplyGitStatus>>,
    /// Held so the debug-only seeding command can write through the same port the application
    /// reads through, rather than injecting a fixture into the view.
    pub cache: Arc<dyn crate::application::ports::workspace_cache::WorkspaceCache>,
    pub register: Arc<crate::application::use_cases::register_workspace::RegisterWorkspace>,
    /// F012's reconciler, for the second trigger: a workspace opened or resumed while connected.
    ///
    /// Needed because the first trigger -- a transition into `Connected` -- fires at startup
    /// before any workspace is open, and nothing reconnects mid-session, so without this
    /// reconciliation would never run at all.
    pub reconcile: Arc<crate::application::use_cases::reconcile::Reconcile>,
}

/// A registered workspace, as the interface sees it.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceDto {
    pub id: String,
    pub name: String,
    /// `"created"` or `"attached"`. The interface shows nothing different, but a developer
    /// reading a log needs to know whether a projection was reused (FR-011).
    pub attachment: String,
}

#[tauri::command]
pub async fn workspace_read_directory(
    workspace_id: String,
    relative_path: String,
    access: State<'_, WorkspaceAccess>,
) -> Result<Vec<EntryDto>, WorkspaceFailure> {
    // Untrusted input: a path that does not parse is refused here rather than being repaired
    // into something that does.
    let path = RelPath::parse(&relative_path).map_err(|_| WorkspaceFailure::Refused)?;
    let page = access
        .provider
        .read_directory(&WorkspaceId(workspace_id), &path, PageRequest::default())
        .await?;
    Ok(page
        .items
        .into_iter()
        .map(|e| EntryDto {
            name: e.name,
            kind: match e.kind {
                apex_protocol::wire::EntryKind::Directory => "directory".into(),
                apex_protocol::wire::EntryKind::File => "file".into(),
            },
            size: e.size,
            modified: e.modified,
        })
        .collect())
}

#[tauri::command]
pub async fn workspace_open(
    name: String,
    host: String,
    base_path: String,
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, crate::adapters::inbound::task_commands::Tasks>,
) -> Result<WorkspaceDto, WorkspaceFailure> {
    use crate::application::use_cases::register_workspace::RegisterWorkspace;
    use crate::domain::workspace::Location;

    // The identity is minted here, client-side, so the workspace is addressable before the
    // engine has ever seen it (A-WORKSPACE). Nothing keys on the display name, which is why two
    // checkouts of one repository can both be called "apex".
    let id = RegisterWorkspace::mint();
    // Kept before `base_path` is moved into the location below.
    let base_path_for_engine = base_path.clone();
    let (ws, attachment) = access
        .register
        .open(
            id,
            name,
            Location::Remote {
                host,
                base: base_path,
            },
        )
        .map_err(|e| WorkspaceFailure::Transport(format!("{e:?}")))?;
    // The engine has to know the workspace before a task can name it. Awaited rather than
    // spawned, so a terminal started immediately after opening a workspace does not race the
    // registration it depends on.
    if let Some(sender) = tasks.sender.as_ref() {
        crate::adapters::inbound::task_commands::register_with_engine(
            sender,
            &ws.id.0,
            &base_path_for_engine,
        )
        .await;
    }
    // **Set whether or not there is an engine** (F012). This used to sit inside the branch above,
    // so an application launched while offline never had a current workspace, and every command
    // that needs one -- reading a cached file, saving, `offline_status` -- refused. That made
    // FR-012, "survives quit and relaunch while still offline", impossible. Which workspace is open
    // is a fact about this client; only the engine's registration depends on there being an engine.
    *tasks.current.lock().expect("current workspace") = Some(ws.id.0.clone());
    reconcile_if_connected(&shell, &access, &ws.id);
    // Recorded in the session, and announced, before the id is given back. Until F006 nothing
    // ever constructed a `WorkspaceReference`, so `workspace:changed` announced `None` forever
    // and the interface had no way to learn which workspace it had just opened -- which is why
    // the file tree was built with a literal id and could only show seeded content.
    shell
        .persist
        .set_workspace(
            &ws.id.0,
            &ws.name,
            crate::domain::session::LocationType::Remote,
        )
        .map_err(|e| WorkspaceFailure::Transport(format!("{e:?}")))?;
    let _ = app.emit("workspace:changed", shell.persist.snapshot().workspace);

    // **Ask once, which is also what subscribes.** The engine starts watching a repository when
    // a client first asks about it, so without this the projection would stay empty and no
    // update would ever be pushed -- a client correct in every part and showing nothing. The
    // answer is applied here rather than awaited by the caller: a workspace that is not a
    // repository answers empty, and neither case is a reason to fail the open (FR-027).
    if let Some(git) = access.git.as_ref() {
        let outcome = git.refresh(&ws.id).await;
        crate::logging::info(&format!("git status on open: {outcome:?}"));
    }

    Ok(WorkspaceDto {
        id: ws.id.0,
        name: ws.name,
        attachment: match attachment {
            crate::application::ports::workspace_cache::Attachment::Created => "created".into(),
            crate::application::ports::workspace_cache::Attachment::Attached => "attached".into(),
        },
    })
}

#[tauri::command]
pub fn workspace_delete(
    workspace_id: String,
    access: State<'_, WorkspaceAccess>,
) -> Result<(), WorkspaceFailure> {
    // Removes cached content **and** the tree, by cascade (FR-012).
    access
        .register
        .delete(&WorkspaceId(workspace_id))
        .map_err(|e| WorkspaceFailure::Transport(format!("{e:?}")))
}

/// Debug builds only: seed a workspace and one listing straight into the projection.
///
/// The end-to-end suite has no engine and no host, so without this the tree renders empty and
/// every assertion about rows — focus, keyboard expansion, indentation — has nothing to stand
/// on. F001 established this pattern with `stub_set_connection`: a command that exists solely so
/// the suite can drive a state, `#[cfg(debug_assertions)]` so it cannot become a production
/// surface by accident.
///
/// It writes through the same `WorkspaceCache` port the application uses, so what the tree then
/// renders came through the real read path rather than a fixture injected into the view.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn workspace_seed_for_tests(
    workspace_id: String,
    parent: String,
    entries: Vec<(String, bool)>,
    access: State<'_, WorkspaceAccess>,
) -> Result<(), WorkspaceFailure> {
    use crate::domain::workspace::{FsEntry, Location, Workspace};

    let id = WorkspaceId(workspace_id);
    let ws = Workspace {
        id: id.clone(),
        name: "seeded".into(),
        location: Location::Remote {
            host: "test".into(),
            base: "/seed".into(),
        },
        last_opened_at: 0,
    };
    access
        .cache
        .register(&ws, 0)
        .map_err(|e| WorkspaceFailure::Transport(format!("{e}")))?;

    let listing: Vec<FsEntry> = entries
        .into_iter()
        .map(|(name, is_dir)| FsEntry {
            name,
            kind: if is_dir {
                apex_protocol::wire::EntryKind::Directory
            } else {
                apex_protocol::wire::EntryKind::File
            },
            size: 0,
            modified: 0,
        })
        .collect();
    let parent = RelPath::parse(&parent).map_err(|_| WorkspaceFailure::Refused)?;
    access
        .cache
        .put_listing(&id, &parent, &listing)
        .map_err(|e| WorkspaceFailure::Transport(format!("{e}")))
}

#[cfg(test)]
mod workspace_tests {
    use super::*;

    #[test]
    fn a_path_that_does_not_parse_is_refused_rather_than_repaired() {
        // The interface could send anything; `..` must not be normalised into something valid.
        for raw in ["../etc/passwd", "/a/../../b"] {
            assert!(RelPath::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn a_gone_workspace_is_distinguishable_from_an_outage() {
        let gone: WorkspaceFailure = ProviderError::WorkspaceGone.into();
        let offline: WorkspaceFailure = ProviderError::Offline.into();
        let a = serde_json::to_string(&gone).unwrap();
        let b = serde_json::to_string(&offline).unwrap();
        assert_ne!(
            a, b,
            "an outage is temporary and the projection is still true; a deleted workspace means \
             the thing being projected does not exist, and the interface must be able to say so"
        );
        assert!(a.contains("gone"), "{a}");
    }

    #[test]
    fn an_unsupported_method_names_the_feature_that_owns_it() {
        use crate::application::ports::workspace_provider::Owner;
        let f: WorkspaceFailure = ProviderError::Unsupported {
            owner: Owner::F006Editor,
        }
        .into();
        let json = serde_json::to_string(&f).unwrap();
        assert!(
            json.contains("F006"),
            "a log must read as a schedule rather than a bug: {json}"
        );
    }
}

// ---- Git (F011) ----

/// One changed path, as the tree reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChangeDto {
    pub path: String,
    /// One of `modified`, `untracked`, `staged`, `deleted`, `conflict`. Lowercase here and
    /// UPPERCASE on the wire, because these are two different boundaries with two different
    /// conventions and pretending otherwise would put a wire spelling in a CSS class name.
    pub status: String,
}

/// Where the repository is, as three cases rather than an optional name.
///
/// A detached head is reported by git as the literal `(detached)` where a name goes, so an
/// optional string would put that text on the status bar as though it were a branch (FR-019).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GitBranchDto {
    Branch { name: String },
    Detached { commit: String },
    None,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusDto {
    pub branch: GitBranchDto,
    pub changes: Vec<GitChangeDto>,
}

/// One file with work the host has not seen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFileDto {
    pub relative_path: String,
    /// Whether the client can merge this file as text. `false` means it will prompt on
    /// reconnection whatever the host did (FR-025a), which the interface says before the
    /// reconnection rather than during it.
    pub mergeable: bool,
}

/// Whether the client is connected, and what it is holding.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineStatusDto {
    pub connected: bool,
    pub pending: Vec<PendingFileDto>,
}

/// Assemble the report from a connection state and the projection.
///
/// **A free function, not a method on a command.** `offline_status` below is a thin wrapper, so
/// this is where the contract's guarantees are testable without Tauri's `State` machinery — the
/// same shape as `parse_region` and `editor_path` above, and the reason F011's git tests exercise
/// `ApplyGitStatus` rather than the command that calls it.
///
/// Guarantee 1 holds because `connected` is derived from the state passed in and from nothing else.
/// Guarantee 2 holds **by signature**: there is no provider here, so this function could not
/// contact the engine if it wanted to. That is stronger than a test counting zero requests, which
/// only says none happened this time.
pub fn offline_report(
    cache: &dyn WorkspaceCache,
    state: &ConnectionState,
    ws: &WorkspaceId,
) -> Result<OfflineStatusDto, WorkspaceFailure> {
    let pending = cache
        .pending_edits(ws)
        .map_err(|_| WorkspaceFailure::UnknownWorkspace)?
        .into_iter()
        .map(|(path, edit)| PendingFileDto {
            relative_path: path.as_str().to_string(),
            mergeable: edit.mergeable,
        })
        .collect();
    Ok(OfflineStatusDto {
        // Only `Connected` is connected. `Connecting` and `Reconnecting` are not: a developer whose
        // save must wait is offline for every purpose this feature has, and reporting them as
        // online would leave the editor claiming a host it cannot reach.
        connected: matches!(state, ConnectionState::Connected),
        pending,
    })
}

/// Path search over what this client holds, with an honest completeness signal.
///
/// **`complete` is the point, not the paths.** FR-007 requires that results not be presented as
/// complete, and the only way the interface can honour that is to be told. A list of paths with no
/// such field would leave every caller free to render it as the whole answer, which is what the
/// requirement forbids — so the flag travels with the results rather than being inferred from the
/// connection state somewhere else.
///
/// `complete` is false whenever the client is disconnected (the projection holds what was cached,
/// not what exists) or the limit was reached (there may be more matches than were returned). Those
/// are different reasons for the same caution, and collapsing them is deliberate: the interface's
/// job is to avoid claiming completeness, not to explain which bound it met.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathSearchDto {
    pub paths: Vec<String>,
    pub complete: bool,
}

/// The developer's own content for a path that carries retained work, if it does.
///
/// **A path with retained work reads as the developer's content** (FR-013). Not the host's last:
/// opening a file that was edited offline and showing what the host has would present their own
/// work as absent, and the next save would then be made against a text they never saw.
///
/// Whole-file only, and unconditional on the connection. Whole-file because a pending edit *is* the
/// whole file -- §4.8 carries content and not a patch -- so serving a range of one would be a window
/// computed for a different copy. Unconditional because pending work exists precisely when the host
/// has not been told, and reconnection is what resolves it; until then this is the file.
///
/// The digest returned is the **base**, never a digest of the pending content. It is what a save is
/// conditional on, so it must stay the content the host confirmed: a digest of the local text would
/// make the engine refuse the eventual write as stale against a version it has never held.
///
/// Extracted from `file_read` so FR-013 is testable without Tauri's `State`, for the reason
/// `offline_report` and `search_complete` are.
pub fn pending_chunk(
    cache: &dyn WorkspaceCache,
    ws: &WorkspaceId,
    rel: &RelPath,
) -> Result<Option<ChunkDto>, WorkspaceFailure> {
    let Some((_, edit)) = cache
        .pending_edits(ws)
        .ok()
        .and_then(|rows| rows.into_iter().find(|(p, _)| p == rel))
    else {
        return Ok(None);
    };
    let text = String::from_utf8(edit.content).map_err(|_| WorkspaceFailure::NotText)?;
    Ok(Some(ChunkDto {
        total: text.len() as u64,
        // A file created offline has no base. An empty digest rather than a fabricated one:
        // `Sha256::parse` refuses it, so a save carrying it is refused at this boundary rather than
        // sent to the engine to be refused there as a phantom conflict.
        sha256: edit
            .base
            .as_ref()
            .map(|(_, hash)| hash.to_string())
            .unwrap_or_default(),
        offset: 0,
        text,
    }))
}

/// Whether a result list may be presented as the whole answer.
///
/// Extracted so FR-007 is testable without an engine: the live suite covers it end to end, but a
/// requirement whose only test needs a real connection is a requirement that goes unchecked on
/// every ordinary run.
pub fn search_complete(connected: bool, returned: usize, asked: u32) -> bool {
    // Disconnected is never complete: the projection holds what was cached, not what exists.
    // Reaching the limit is never complete either, connected or not, because there may be more
    // matches than were asked for.
    connected && (returned as u32) < asked
}

/// Search cached paths. Never contacts the engine, connected or not (FR-031, C6).
///
/// The use case behind this has existed since F005 and nothing could reach it: no command, no
/// registration, no caller. F012 needs it reachable because §11.3 makes path search a capability
/// that degrades offline rather than one that disappears.
#[tauri::command]
pub fn workspace_search_paths(
    fragment: String,
    limit: Option<u32>,
    shell: State<'_, Shell>,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<PathSearchDto, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    let asked = limit.unwrap_or(crate::application::use_cases::search_paths::DEFAULT_LIMIT);
    let search =
        crate::application::use_cases::search_paths::SearchPaths::new(access.cache.clone());
    let paths = search
        .find(&ws, &fragment, Some(asked))
        .map_err(|_| WorkspaceFailure::UnknownWorkspace)?;
    let connected = matches!(shell.connection.current(), ConnectionState::Connected);
    Ok(PathSearchDto {
        complete: search_complete(connected, paths.len(), asked),
        paths: paths.iter().map(|p| p.as_str().to_string()).collect(),
    })
}

/// What the interface reads to know it is offline and what is held locally.
///
/// Takes `Shell` for the connection state rather than carrying a second handle on
/// `WorkspaceAccess`. The connection is one fact for the application, not one per workspace — §13.1
/// and spec.md's Assumptions both say offline is a state of the connection and not of the workspace
/// — so `Shell` is where it belongs and already is. `session_get` above takes two states the same
/// way.
#[tauri::command]
pub fn offline_status(
    shell: State<'_, Shell>,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<OfflineStatusDto, WorkspaceFailure> {
    // Resolved in the core, never accepted from the view: a workspace id from the bridge is
    // untrusted input, and this command's answer includes the paths a developer is working on.
    let ws = current_workspace(&tasks)?;
    offline_report(access.cache.as_ref(), &shell.connection.current(), &ws)
}

/// The git state this client has for the current workspace.
///
/// **Reads the projection; never the engine.** An outage therefore costs nothing and times out
/// never, and the last state the client knew stays on screen rather than clearing (FR-029).
///
/// The workspace is resolved in the core, not accepted from the webview. A workspace identity
/// arriving from the view is an identity the view could choose, which is the shape F010 and
/// F006 both refused for the same reason (Principle VI).
#[tauri::command]
pub async fn git_status(
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<GitStatusDto, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    let git = access
        .cache
        .git_status(&ws)
        .map_err(|_| WorkspaceFailure::UnknownWorkspace)?;
    Ok(GitStatusDto {
        branch: match git.branch {
            apex_protocol::wire::BranchPosition::Branch(name) => GitBranchDto::Branch { name },
            apex_protocol::wire::BranchPosition::Detached(commit) => {
                GitBranchDto::Detached { commit }
            }
            apex_protocol::wire::BranchPosition::None => GitBranchDto::None,
        },
        changes: git
            .changes
            .into_iter()
            .map(|c| GitChangeDto {
                path: c.path,
                status: status_slug(&c.status).to_string(),
            })
            .collect(),
    })
}

fn status_slug(status: &apex_protocol::wire::GitStatusKind) -> &'static str {
    use apex_protocol::wire::GitStatusKind::*;
    match status {
        Modified => "modified",
        Untracked => "untracked",
        Staged => "staged",
        Deleted => "deleted",
        Conflict => "conflict",
    }
}

// ---- Watching (F004's wiring, which F011 depends on) ----

/// What a watch request achieved, as the interface sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchOutcomeDto {
    /// The size of the requested set after the call, **not** the number of host descriptors.
    pub watching: u32,
    /// Paths the host could not watch. Data rather than an error: FR-005a keeps the workspace
    /// browsable when watching fails, and FR-005 requires the loss be stated rather than silent.
    pub refused: Vec<String>,
}

/// Ask the engine to watch these paths, and to stop watching those.
///
/// **One command taking both halves**, because they are one intention: the interface declares
/// the set it wants watched, and the difference is what travels. Two commands would let a
/// client send an add without its matching remove and drift out of step with the engine a
/// folder at a time.
///
/// Folder paths for expanded folders and **file** paths for open editors, exactly as the port
/// documents: the engine derives the directories. A caller that resolved that itself could not
/// tell unwatching on a collapse from unwatching on a tab close, so collapsing a folder would
/// silently stop reporting a file still open inside it (FR-003c, A-WATCHSCOPE).
#[tauri::command]
pub async fn workspace_watch(
    add: Vec<String>,
    remove: Vec<String>,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<WatchOutcomeDto, WorkspaceFailure> {
    // Resolved in the core, never accepted from the webview (Principle VI).
    let ws = current_workspace(&tasks)?;

    let mut watching = 0u32;
    let mut refused: Vec<String> = Vec::new();

    if !remove.is_empty() {
        let paths = parse_paths(&remove)?;
        // Unwatched first. Doing it the other way round means the peak set is the union of
        // before and after, which on a large collapse-and-expand is twice what was ever wanted.
        let outcome = access.provider.unwatch(&ws, &paths).await?;
        watching = outcome.watching;
    }
    if !add.is_empty() {
        let paths = parse_paths(&add)?;
        let outcome = access.provider.watch(&ws, &paths).await?;
        watching = outcome.watching;
        refused = outcome
            .refused
            .into_iter()
            .map(|r| r.path.as_str().to_string())
            .collect();
    }
    Ok(WatchOutcomeDto { watching, refused })
}

/// Reconcile a workspace that has just become current, if the client is connected.
///
/// **The second trigger** (A-RECONNECT, amended by F012's implementation). The first -- a
/// transition into `Connected` -- is subscribed in the composition root, and it fires once at
/// startup *before* any workspace is open, so it finds nothing to reconcile. With no reconnection
/// loop in the client (§11.5's is unbuilt), that was the only transition there would ever be:
/// reconciliation never ran. Opening or resuming a workspace while connected is the moment both
/// inputs exist -- a connection and a workspace -- and it mirrors FR-029b's two triggers for
/// prefetch exactly.
///
/// Spawned, not awaited: opening a workspace must not wait on writing every pending file.
fn reconcile_if_connected(shell: &Shell, access: &WorkspaceAccess, ws: &WorkspaceId) {
    if !matches!(shell.connection.current(), ConnectionState::Connected) {
        return;
    }
    let reconcile = access.reconcile.clone();
    let ws = ws.clone();
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            crate::logging::warn("could not reconcile offline work: no runtime");
            return;
        };
        rt.block_on(async move {
            let report = reconcile.run(&ws).await;
            for (path, outcome) in &report.files {
                crate::logging::info(&format!("reconcile {}: {outcome:?}", path.as_str()));
            }
        });
    });
}

/// Untrusted input, refused rather than repaired into something that parses.
fn parse_paths(raw: &[String]) -> Result<Vec<RelPath>, WorkspaceFailure> {
    raw.iter().map(|p| editor_path(p)).collect()
}

/// Make a restored workspace live again.
///
/// **A relaunch commonly meets an engine that has never heard of this workspace.** The engine
/// outlives its client, but only while it has something to preserve: with no tasks running it
/// exits when the last client goes (A-ENGINELIFE rule 3). So the ordinary case is a fresh
/// engine, and everything `workspace_open` told it has to be said again -- registration first,
/// because a watch and a status both name a workspace the engine must already know.
///
/// Without this a restored session looked correct and was inert: the tree showed its cached
/// listing, git showed its stored marks, and nothing would ever update either of them again.
/// That is the most misleading state this application can be in, because it is
/// indistinguishable from a host where nothing has changed.
///
/// Idempotent, and safe against an engine that *does* still know the workspace: registration is
/// keyed on the identity, and asking for a status the engine already has costs one computation.
#[tauri::command]
pub async fn workspace_resume(
    workspace_id: String,
    shell: State<'_, Shell>,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<bool, WorkspaceFailure> {
    let ws = WorkspaceId(workspace_id);
    // The root path is not in the session -- it records an identity and a display name -- so it
    // comes from this client's own projection, which is where the workspace was recorded when
    // it was opened.
    let Some(known) = access
        .cache
        .workspace(&ws)
        .map_err(|e| WorkspaceFailure::Transport(format!("{e:?}")))?
    else {
        // Nothing to resume. Not an error: a session naming a workspace this client no longer
        // holds is what a deleted workspace leaves behind.
        return Ok(false);
    };
    let base = match &known.location {
        crate::domain::workspace::Location::Remote { base, .. } => base.clone(),
        crate::domain::workspace::Location::Local { base } => base.clone(),
    };

    if let Some(sender) = tasks.sender.as_ref() {
        crate::adapters::inbound::task_commands::register_with_engine(sender, &ws.0, &base).await;
    }
    // Outside the branch for the reason `workspace_open` gives: a restored session launched offline
    // must still know which workspace it is showing, or nothing in it can be read (FR-012).
    *tasks.current.lock().expect("current workspace") = Some(ws.0.clone());
    reconcile_if_connected(&shell, &access, &ws);
    // Asking is also what makes the engine start watching this repository, so this is not
    // merely a refresh: without it nothing would be pushed for the rest of the session.
    if let Some(git) = access.git.as_ref() {
        let outcome = git.refresh(&ws).await;
        crate::logging::info(&format!("git status on resume: {outcome:?}"));
    }
    Ok(true)
}

/// Line coordinates only, for one file.
///
/// **There is no field here for content and there must never be one** (§12.3, FR-021). The
/// guarantee is asserted on the payload in `git_diff.rs`, because a caller that discards text
/// and a result that carries it look identical from the caller's side.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffDto {
    pub added: Vec<[u32; 2]>,
    pub modified: Vec<[u32; 2]>,
    /// Positions where lines were removed. A position, not a range: the removed lines are not
    /// in this file, so there is nothing to draw a range over.
    pub deleted: Vec<u32>,
}

/// Which lines of one file differ.
///
/// Goes to the engine rather than to the projection, unlike `git_status`: a diff is per file
/// and per open editor, so caching every one would hold the whole repository's diffs for the
/// sake of the one file on screen. An outage therefore makes this unavailable, which is the
/// truth -- and the gutter shows nothing rather than stale marks.
#[tauri::command]
pub async fn git_file_diff(
    path: String,
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<GitDiffDto, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    // Contained here as well as on the engine. A path from the webview is untrusted exactly as
    // a path from the wire is (Principle VI).
    let rel = editor_path(&path)?;
    let Some(git) = access.git.as_ref() else {
        // No engine configured. An empty diff rather than an error: a gutter with no marks is
        // the correct rendering of "nothing is known", and a failure would put an error beside
        // a file the developer can read perfectly well.
        return Ok(GitDiffDto::default());
    };
    let diff = git.file_diff(&ws, rel.as_str()).await?;
    Ok(GitDiffDto {
        added: diff.added,
        modified: diff.modified,
        deleted: diff.deleted,
    })
}
