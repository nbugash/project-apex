//! The `ShellCommands` inbound adapter.
//!
//! Input arriving here is untrusted regardless of interface-layer validation (Constitution
//! Principle VI). Every argument is validated in the core, which rejects rather than coerces.

use crate::application::error::ShellError;
use crate::application::use_cases::observe_connection::ObserveConnection;
use crate::application::use_cases::persist_session::PersistSession;
use crate::domain::layout::RegionId;
use crate::domain::rail::{DestinationId, RailCatalogue, ToolWindowState};
use crate::domain::session::{DocumentId, SessionSnapshot};
use crate::window::controller::WindowController;
use std::sync::Arc;
use tauri::State;

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

#[tauri::command]
pub fn documents_open(
    display_name: String,
    shell: State<'_, Shell>,
) -> Result<DocumentId, ShellError> {
    shell.persist.open_document(&display_name)
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
use crate::domain::workspace::{PageRequest, RelPath, WorkspaceId};
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
pub fn workspace_open(
    name: String,
    host: String,
    base_path: String,
    access: State<'_, WorkspaceAccess>,
) -> Result<WorkspaceDto, WorkspaceFailure> {
    use crate::application::use_cases::register_workspace::RegisterWorkspace;
    use crate::domain::workspace::Location;

    // The identity is minted here, client-side, so the workspace is addressable before the
    // engine has ever seen it (A-WORKSPACE). Nothing keys on the display name, which is why two
    // checkouts of one repository can both be called "apex".
    let id = RegisterWorkspace::mint();
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
