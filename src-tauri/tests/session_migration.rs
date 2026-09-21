//! Integration: an existing session survives the upgrade that added tool window state.
//!
//! The unit tests exercise serde directly. This goes through the real store and the real
//! restore path, which is what actually runs when a user upgrades. Without this behaviour,
//! shipping F018 would silently erase every existing user's window geometry, layout and
//! open tabs — the store previously discarded any file whose schema version was not
//! exactly current.

use apex_shell::adapters::outbound::json_session_store::JsonFileSessionStore;
use apex_shell::application::ports::session_store::SessionStore;
use apex_shell::application::use_cases::restore_session::RestoreSession;
use apex_shell::domain::geometry::DisplayBounds;
use apex_shell::domain::rail::{DestinationId, ToolWindowState};
use apex_shell::domain::session::{PersistedSession, SCHEMA_VERSION};
use std::sync::Arc;

/// Exactly what the previous release wrote: schema version 1, no tool window field.
const VERSION_ONE_SESSION: &str = r#"{
  "schema_version": 1,
  "workspace": { "name": "payments-platform", "location_type": "REMOTE" },
  "window": { "x": 240, "y": 160, "width": 1100, "height": 760, "maximized": false },
  "layout": {
    "navigation": { "visible": true, "extent": 300 },
    "output": { "visible": false, "extent": 220 },
    "document_area": { "visible": true, "extent": 0 }
  },
  "documents": [
    { "id": "doc-a", "display_name": "main.rs", "order": 0 },
    { "id": "doc-b", "display_name": "lib.rs", "order": 1 }
  ],
  "focused_document_id": "doc-b"
}"#;

fn screen() -> Vec<DisplayBounds> {
    vec![DisplayBounds {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }]
}

fn restore_from(body: &str) -> (tempfile::TempDir, PersistedSession) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    std::fs::write(&path, body).unwrap();
    let restored =
        RestoreSession::new(Arc::new(JsonFileSessionStore::new(path))).execute(&screen());
    (dir, restored)
}

#[test]
fn a_version_one_session_keeps_everything_the_user_had() {
    let (_dir, restored) = restore_from(VERSION_ONE_SESSION);

    assert_eq!(restored.window.width, 1100, "window geometry survived");
    assert_eq!(restored.window.x, 240);
    assert_eq!(restored.layout.navigation.extent, 300, "layout survived");
    assert!(
        !restored.layout.output.visible,
        "hidden region stayed hidden"
    );
    assert_eq!(restored.documents.len(), 2, "open tabs survived");
    assert_eq!(
        restored.focused_document_id,
        Some(DocumentIdShim::b()),
        "focus survived"
    );
    assert_eq!(
        restored.workspace.as_ref().map(|w| w.name.as_str()),
        Some("payments-platform"),
        "workspace reference survived"
    );
}

#[test]
fn the_new_field_arrives_with_defaults() {
    let (_dir, restored) = restore_from(VERSION_ONE_SESSION);
    assert_eq!(restored.tool_window, ToolWindowState::default());
}

#[test]
fn the_migrated_session_is_rewritten_at_the_current_version() {
    let (dir, restored) = restore_from(VERSION_ONE_SESSION);
    assert_eq!(restored.schema_version, SCHEMA_VERSION);

    // And the migration happens once: saving then reloading yields the current version
    // without another pass.
    let store = JsonFileSessionStore::new(dir.path().join("session.json"));
    store.save(&restored).unwrap();
    let reloaded = store.load().expect("the rewritten file loads");
    assert_eq!(reloaded.schema_version, SCHEMA_VERSION);
    assert_eq!(reloaded.window.width, 1100, "still the user's geometry");
}

#[test]
fn a_future_version_is_still_discarded_rather_than_guessed_at() {
    let future = VERSION_ONE_SESSION.replace("\"schema_version\": 1", "\"schema_version\": 99");
    let (_dir, restored) = restore_from(&future);

    // Defaults, not the file's contents: this build cannot interpret a shape it does not
    // know, and guessing risks corrupting it on the next write.
    assert_eq!(
        restored.window.width,
        PersistedSession::default().window.width
    );
    assert!(restored.documents.is_empty());
}

#[test]
fn a_dangling_active_destination_does_not_cost_the_user_their_session() {
    let with_ghost = VERSION_ONE_SESSION.replace(
        "\"focused_document_id\": \"doc-b\"",
        "\"focused_document_id\": \"doc-b\",\n  \"tool_window\": { \"active_destination_id\": \"removed-in-a-later-version\", \"collapsed\": false, \"width\": 300 }",
    );
    let (_dir, restored) = restore_from(&with_ghost);

    assert_eq!(
        restored.documents.len(),
        2,
        "the session is repaired, not discarded"
    );
    assert_ne!(
        restored.tool_window.active_destination_id,
        Some(DestinationId::new("removed-in-a-later-version")),
        "the dangling reference was replaced"
    );
    assert!(restored.tool_window.active_destination_id.is_some());
}

/// Small helper so the assertion above reads clearly.
struct DocumentIdShim;
impl DocumentIdShim {
    fn b() -> apex_shell::domain::session::DocumentId {
        apex_shell::domain::session::DocumentId("doc-b".into())
    }
}
