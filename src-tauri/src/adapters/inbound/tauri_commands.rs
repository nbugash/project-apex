//! The `ShellCommands` inbound adapter.
//!
//! Input arriving here is untrusted regardless of interface-layer validation (Constitution
//! Principle VI). Every argument is validated in the core, which rejects rather than coerces.

use crate::application::error::ShellError;
use crate::application::use_cases::observe_connection::ObserveConnection;
use crate::application::use_cases::persist_session::PersistSession;
use crate::domain::layout::RegionId;
use crate::domain::session::{DocumentId, SessionSnapshot};
use crate::window::controller::WindowController;
use std::sync::Arc;
use tauri::State;

pub struct Shell {
    pub persist: Arc<PersistSession>,
    pub connection: Arc<ObserveConnection>,
    pub window: Arc<WindowController>,
    /// Debug builds only: lets the end-to-end suite drive connection transitions. Absent
    /// from release builds, so it cannot become a production surface by accident.
    #[cfg(debug_assertions)]
    pub stub: Arc<crate::adapters::outbound::stub_connection::StubConnectionStatusSource>,
}

/// Region identifiers arrive as strings from the bridge and are not trusted to be valid.
fn parse_region(raw: &str) -> Result<RegionId, ShellError> {
    match raw {
        "navigation" => Ok(RegionId::Navigation),
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
    fn known_region_identifiers_parse() {
        assert_eq!(parse_region("navigation"), Ok(RegionId::Navigation));
        assert_eq!(parse_region("output"), Ok(RegionId::Output));
        assert_eq!(parse_region("document_area"), Ok(RegionId::DocumentArea));
    }
}
