//! The `ShellCommands` inbound adapter.
//!
//! Input arriving here is untrusted regardless of interface-layer validation (Constitution
//! Principle VI). Every argument is validated in the core, which rejects rather than coerces.

use crate::application::error::ShellError;
use crate::adapters::inbound::task_commands::Tasks;
use crate::application::use_cases::edit_file::{EditFile, WriteOutcome};
use crate::application::use_cases::observe_connection::ObserveConnection;
use crate::application::use_cases::persist_session::PersistSession;
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

#[tauri::command]
pub fn session_get(shell: State<'_, Shell>) -> SessionSnapshot {
    shell.persist.snapshot()
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

/// What a save produced, as four cases the interface switches on.
///
/// Carried in the success channel deliberately. Three of these are failures, but they are
/// failures the interface must *branch* on rather than merely report, and splitting them across
/// `Ok` and `Err` would push the caller back to inspecting an error to find out which it was --
/// which is what the typed variants exist to prevent.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WriteOutcomeDto {
    Written { sha256: String },
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
    access: State<'_, WorkspaceAccess>,
    tasks: State<'_, Tasks>,
) -> Result<WriteOutcomeDto, WorkspaceFailure> {
    let ws = current_workspace(&tasks)?;
    let rel = editor_path(&path)?;
    // A base that is not a digest cannot have come from a read this client performed. Refused
    // rather than forwarded: the engine would compare it, fail to match and answer `-32004`,
    // and the developer would be told a colleague edited their file when nobody did.
    let base = Sha256::parse(&base).ok_or(WorkspaceFailure::Refused)?;

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
    /// Held so the debug-only seeding command can write through the same port the application
    /// reads through, rather than injecting a fixture into the view.
    pub cache: Arc<dyn crate::application::ports::workspace_cache::WorkspaceCache>,
    pub register: Arc<crate::application::use_cases::register_workspace::RegisterWorkspace>,
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
        *tasks.current.lock().expect("current workspace") = Some(ws.id.0.clone());
    }
    // Recorded in the session, and announced, before the id is given back. Until F006 nothing
    // ever constructed a `WorkspaceReference`, so `workspace:changed` announced `None` forever
    // and the interface had no way to learn which workspace it had just opened -- which is why
    // the file tree was built with a literal id and could only show seeded content.
    shell
        .persist
        .set_workspace(&ws.id.0, &ws.name, crate::domain::session::LocationType::Remote)
        .map_err(|e| WorkspaceFailure::Transport(format!("{e:?}")))?;
    let _ = app.emit("workspace:changed", shell.persist.snapshot().workspace);

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
