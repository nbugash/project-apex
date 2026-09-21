//! Integration: the session store against real files, including every corruption case the
//! specification says must not prevent launch (FR-008, SC-003).

use apex_shell::adapters::outbound::json_session_store::JsonFileSessionStore;
use apex_shell::application::ports::session_store::SessionStore;
use apex_shell::application::use_cases::restore_session::RestoreSession;
use apex_shell::domain::geometry::DisplayBounds;
use apex_shell::domain::session::PersistedSession;
use std::sync::Arc;

fn primary() -> Vec<DisplayBounds> {
    vec![DisplayBounds {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }]
}

fn store_in(dir: &tempfile::TempDir) -> JsonFileSessionStore {
    JsonFileSessionStore::new(dir.path().join("session.json"))
}

#[test]
fn round_trips_a_session_through_a_real_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    let mut session = PersistedSession::default();
    session.open_document("main.rs").unwrap();
    session.open_document("lib.rs").unwrap();

    store.save(&session).unwrap();
    let loaded = store.load().expect("saved session loads");
    assert_eq!(loaded, session);
    assert!(loaded.is_coherent());
}

#[test]
fn an_absent_file_is_absence_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(store_in(&dir).load().is_none());
}

#[test]
fn every_corruption_case_yields_defaults_rather_than_failing() {
    // FR-008: discarded in full, never partially recovered.
    let cases: [(&str, &str); 4] = [
        ("malformed json", "not json at all"),
        ("truncated", r#"{"schema_version": 1, "window": {"#),
        ("empty file", ""),
        ("wrong shape", r#"{"unexpected": true}"#),
    ];
    for (label, body) in cases {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        std::fs::write(dir.path().join("session.json"), body).unwrap();

        let restored = RestoreSession::new(Arc::new(store)).execute(&primary());
        assert_eq!(
            restored,
            PersistedSession::default().repaired(&primary()),
            "{label} should yield defaults"
        );
    }
}

#[test]
fn a_dangling_focus_reference_invalidates_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    let mut session = PersistedSession::default();
    session.open_document("a.rs").unwrap();
    // Valid JSON, valid schema, incoherent: focus names a document that is not present.
    session.documents.clear();
    store.save(&session).unwrap();

    let restored = RestoreSession::new(Arc::new(store_in(&dir))).execute(&primary());
    assert!(restored.documents.is_empty());
    assert!(
        restored.focused_document_id.is_none(),
        "defaults, not partial recovery"
    );
}

#[test]
fn a_future_schema_version_is_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    let mut session = PersistedSession::default();
    session.schema_version = 99;
    store.save(&session).unwrap();

    let restored = RestoreSession::new(Arc::new(store_in(&dir))).execute(&primary());
    assert_eq!(
        restored.schema_version,
        apex_shell::domain::session::SCHEMA_VERSION
    );
}

#[test]
fn saving_leaves_no_temporary_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    store.save(&PersistedSession::default()).unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "write-then-rename must not leak temp files"
    );
}
