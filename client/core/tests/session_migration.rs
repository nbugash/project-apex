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
use apex_shell::domain::rail::{DestinationId, RailCatalogue, ToolWindowState};
use apex_shell::domain::session::{PersistedSession, PersistedTask, SCHEMA_VERSION};
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
    // The v1 file's left panel was a generic `navigation` region; it is the tool window
    // now. The width is what the user actually chose, so it survives the rename rather
    // than resetting to a default.
    assert_eq!(restored.tool_window.width, 300, "panel width survived");
    assert!(
        restored.layout.legacy_navigation.is_none(),
        "the legacy region is folded in, not carried alongside"
    );
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
fn the_new_field_arrives_ready_to_use() {
    let (_dir, restored) = restore_from(VERSION_ONE_SESSION);
    // Defaulted AND repaired. A migrated file that arrives with no active destination
    // would render a rail where nothing looks selected, which is not a state the
    // prototype has.
    // The width comes from the v1 file's retired `navigation` region, not from the
    // default: it is the size the user chose for the same panel.
    assert_eq!(restored.tool_window.width, 300);
    assert_ne!(restored.tool_window.width, ToolWindowState::default().width);
    assert!(!restored.tool_window.collapsed);
    assert_eq!(
        restored.tool_window.active_destination_id,
        RailCatalogue::default()
            .first_available()
            .map(|d| d.id.clone()),
        "the migrated panel opens on the first available destination"
    );
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

/// Exactly what the release before F010 wrote: schema version 2, tool window but no tasks.
const VERSION_TWO_SESSION: &str = r#"{
  "schema_version": 2,
  "workspace": { "name": "payments-platform", "location_type": "REMOTE" },
  "window": { "x": 100, "y": 80, "width": 1440, "height": 900, "maximized": false },
  "layout": { "output": { "visible": true, "extent": 268 },
              "document_area": { "visible": true, "extent": 600 } },
  "documents": [],
  "focused_document_id": null,
  "tool_window": { "active_destination_id": "project", "collapsed": false, "width": 310 }
}"#;

#[test]
fn a_store_written_before_tasks_existed_still_loads() {
    // The migration, and the reason `serde(default)` is the mechanism rather than a version
    // check somebody has to remember to extend. A user upgrading into F010 has a version 2 file
    // and no tasks; refusing it would erase their geometry, layout and tabs to add a field they
    // have no value for yet.
    let (_dir, restored) = restore_from(VERSION_TWO_SESSION);

    assert_eq!(
        restored.workspace.as_ref().map(|w| w.name.as_str()),
        Some("payments-platform"),
        "the upgrade discarded the workspace"
    );
    assert_eq!(
        restored.window.width, 1440,
        "the upgrade discarded the geometry"
    );
    assert_eq!(
        restored.tool_window.width, 310,
        "the upgrade discarded the tool window the previous version stored"
    );
    assert!(
        restored.tasks.is_empty(),
        "a store with no tasks produced some from nowhere"
    );
}

#[test]
fn a_stored_task_identity_survives_a_client_restart() {
    // FR-031d and A-STATE2. A task outlives the connection that started it, and `attach` reaches
    // one by an identity the client must already know -- so the identity has to survive the
    // restart or reattachment works only for a client that never closed.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.json");
    let store = JsonFileSessionStore::new(path.clone());

    let session = PersistedSession {
        schema_version: SCHEMA_VERSION,
        tasks: vec![
            PersistedTask {
                task_id: "build-01".into(),
                workspace_id: "ws1".into(),
            },
            PersistedTask {
                task_id: "test-02".into(),
                workspace_id: "ws1".into(),
            },
        ],
        ..PersistedSession::default()
    };
    store.save(&session).expect("save");

    // A **new** store over the same file, which is what a restarted client has: nothing in
    // memory, only what is on disk.
    let reopened = JsonFileSessionStore::new(path);
    let loaded = reopened.load().expect("load");

    let ids: Vec<&str> = loaded.tasks.iter().map(|t| t.task_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["build-01", "test-02"],
        "the identities did not survive"
    );
    assert_eq!(loaded.tasks[0].workspace_id, "ws1");
}

#[test]
fn the_store_carries_the_identity_and_nothing_else_about_a_task() {
    // FR-005a's boundary, checked on the bytes rather than on the type. A command would put a
    // credential passed in argv on disk; output would make the store grow without bound for a
    // client that never returns.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.json");
    let store = JsonFileSessionStore::new(path.clone());

    let session = PersistedSession {
        tasks: vec![PersistedTask {
            task_id: "build-01".into(),
            workspace_id: "ws1".into(),
        }],
        ..PersistedSession::default()
    };
    store.save(&session).expect("save");

    let raw = std::fs::read_to_string(&path).expect("read");
    assert!(
        raw.contains("build-01"),
        "the identity was not stored: {raw}"
    );
    assert!(
        !raw.contains("\"command\""),
        "the store carried a task's command: {raw}"
    );
    assert!(
        !raw.contains("\"env\""),
        "the store carried a task's environment: {raw}"
    );
}

#[test]
fn a_client_that_lost_its_store_entirely_has_nothing_to_reattach_to() {
    // SC-023's second half, and it has to **discard** the store rather than ignore it. A harness
    // that kept the identities in a variable and called `list` for form's sake would be testing
    // nothing: the whole claim is that a client with no record of its tasks can still find them,
    // and a client that secretly still has the record has not lost anything.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.json");
    let store = JsonFileSessionStore::new(path.clone());

    let session = PersistedSession {
        tasks: vec![PersistedTask {
            task_id: "build-01".into(),
            workspace_id: "ws1".into(),
        }],
        ..PersistedSession::default()
    };
    store.save(&session).expect("save");
    assert!(path.exists());

    // Gone, as after a profile wipe or a move to a new machine.
    std::fs::remove_file(&path).expect("remove");

    let reopened = JsonFileSessionStore::new(path);
    let loaded = reopened.load().unwrap_or_default();
    assert!(
        loaded.tasks.is_empty(),
        "the store was supposed to be gone and still produced identities"
    );
    // Which is precisely why `execution/list` exists: without it these tasks keep running and
    // are unreachable until the instance idles out (SC-023).
}

/// Exactly what the previous release wrote: schema version 3, no autosave field.
const VERSION_THREE_SESSION: &str = r#"{
  "schema_version": 3,
  "workspace": { "name": "payments-platform", "location_type": "REMOTE" },
  "window": { "x": 240, "y": 160, "width": 1100, "height": 760, "maximized": false },
  "layout": {
    "navigation": { "visible": true, "extent": 300 },
    "output": { "visible": false, "extent": 220 },
    "document_area": { "visible": true, "extent": 0 }
  },
  "documents": [{ "id": "doc-a", "display_name": "main.rs", "order": 0 }],
  "focused_document_id": "doc-a",
  "tasks": []
}"#;

#[test]
fn a_profile_that_never_chose_autosave_does_not_get_it_switched_on() {
    // The migration A-STATE2 established, applied to a field where the default is not merely
    // convenient. Autosave writes the developer's file without being asked, and an upgrade is
    // not consent. `false` is the safe value *and* what `serde(default)` produces, which is the
    // only reason relying on the default is defensible here.
    let parsed: PersistedSession =
        serde_json::from_str(VERSION_THREE_SESSION).expect("a version 3 store must still parse");

    assert!(
        !parsed.autosave,
        "an upgrade must not start writing a developer's files for them"
    );
    assert_eq!(
        parsed.documents.len(),
        1,
        "the rest of the store must survive the upgrade"
    );
    assert_eq!(
        SCHEMA_VERSION, 4,
        "this test describes the upgrade into version 4; a later version needs its own"
    );
}
