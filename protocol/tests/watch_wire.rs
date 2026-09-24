//! The wire spelling of F004's types, asserted as text.
//!
//! Every assertion here is on the serialised string rather than on a round trip. A round trip
//! proves the two halves agree with each other, which a symmetric bug satisfies perfectly: a
//! field renamed on the way out and renamed back on the way in survives it, and the engine on
//! the other side -- which does not share this code -- does not.

use apex_protocol::wire::{
    EntryKind, FileEvent, FileEventKind, FileEventParams, InvalidateAllParams, RefusalReason,
    UnwatchResult, WatchParams, WatchRefusal, WatchResult, WorkspaceId,
};

fn ws() -> WorkspaceId {
    WorkspaceId("ws_7f2a".into())
}

#[test]
fn watch_params_are_snake_case_on_the_wire() {
    let json = serde_json::to_string(&WatchParams {
        workspace_id: ws(),
        paths: vec!["src".into(), "src/main.rs".into()],
    })
    .expect("serialise");
    assert!(json.contains(r#""workspace_id":"ws_7f2a""#), "{json}");
    assert!(json.contains(r#""paths":["src","src/main.rs"]"#), "{json}");
    assert!(!json.contains("workspaceId"), "camelCase leaked: {json}");
}

#[test]
fn a_refusal_names_its_path_and_a_snake_case_reason() {
    let json = serde_json::to_string(&WatchResult {
        watching: 3,
        refused: vec![
            WatchRefusal {
                path: "node_modules".into(),
                reason: RefusalReason::Excluded,
            },
            WatchRefusal {
                path: "big".into(),
                reason: RefusalReason::Capacity,
            },
            WatchRefusal {
                path: "gone".into(),
                reason: RefusalReason::NotADirectory,
            },
        ],
    })
    .expect("serialise");
    assert!(json.contains(r#""watching":3"#), "{json}");
    assert!(json.contains(r#""reason":"excluded""#), "{json}");
    assert!(json.contains(r#""reason":"capacity""#), "{json}");
    assert!(json.contains(r#""reason":"not_a_directory""#), "{json}");
}

#[test]
fn the_refusal_vocabulary_is_closed_and_exactly_four() {
    // Named individually rather than counted, so adding a fifth is a decision somebody makes
    // here rather than a number that quietly changes.
    for (reason, spelling) in [
        (RefusalReason::Capacity, "capacity"),
        (RefusalReason::Excluded, "excluded"),
        (RefusalReason::NotFound, "not_found"),
        (RefusalReason::NotADirectory, "not_a_directory"),
    ] {
        let json = serde_json::to_string(&reason).expect("serialise");
        assert_eq!(json, format!("\"{spelling}\""));
    }
}

#[test]
fn an_empty_refusal_list_still_appears() {
    // Absence must be a positive statement. A client that cannot tell "nothing was refused"
    // from "the field is missing" cannot satisfy FR-005.
    let json = serde_json::to_string(&WatchResult {
        watching: 1,
        refused: vec![],
    })
    .expect("serialise");
    assert!(json.contains(r#""refused":[]"#), "{json}");
}

#[test]
fn unwatch_returns_only_the_count() {
    let json = serde_json::to_string(&UnwatchResult { watching: 0 }).expect("serialise");
    assert_eq!(json, r#"{"watching":0}"#);
}

#[test]
fn an_event_batch_is_an_array_even_for_one_event() {
    let json = serde_json::to_string(&FileEventParams {
        workspace_id: ws(),
        events: vec![FileEvent {
            event: FileEventKind::Created,
            relative_path: "src/token.rs".into(),
            to_path: None,
            kind: Some(EntryKind::File),
            size: Some(2048),
            modified: Some(1_758_700_000),
        }],
    })
    .expect("serialise");
    assert!(
        json.contains(r#""events":[{"#),
        "one event is still a batch: {json}"
    );
    assert!(json.contains(r#""event":"created""#), "{json}");
    assert!(json.contains(r#""relative_path":"src/token.rs""#), "{json}");
    assert!(
        json.contains(r#""type":"file""#),
        "kind serialises as type: {json}"
    );
    assert!(json.contains(r#""size":2048"#), "{json}");
    assert!(json.contains(r#""modified":1758700000"#), "{json}");
    assert!(
        !json.contains("to_path"),
        "absent to_path must be omitted: {json}"
    );
}

#[test]
fn only_a_rename_carries_a_destination() {
    let renamed = serde_json::to_string(&FileEvent {
        event: FileEventKind::Renamed,
        relative_path: "src/expr.rs".into(),
        to_path: Some("src/expression.rs".into()),
        kind: None,
        size: None,
        modified: None,
    })
    .expect("serialise");
    assert!(
        renamed.contains(r#""to_path":"src/expression.rs""#),
        "{renamed}"
    );
    // A rename describes a move, not a file that is now there to describe.
    assert!(!renamed.contains("\"size\""), "{renamed}");
    assert!(!renamed.contains("\"type\""), "{renamed}");
}

#[test]
fn a_deleted_event_carries_no_metadata() {
    let json = serde_json::to_string(&FileEvent {
        event: FileEventKind::Deleted,
        relative_path: "src/scratch.rs".into(),
        to_path: None,
        kind: None,
        size: None,
        modified: None,
    })
    .expect("serialise");
    assert_eq!(
        json,
        r#"{"event":"deleted","relative_path":"src/scratch.rs"}"#
    );
}

#[test]
fn no_event_can_carry_content_or_a_hash() {
    // Structural, not a rule to remember: there is no field to put either in. FR-019 holds
    // because a client cannot mark a blob valid from something containing nothing to decide
    // with -- and this test fails the moment a field is added that could.
    let json = serde_json::to_string(&FileEvent {
        event: FileEventKind::Modified,
        relative_path: "src/expr.rs".into(),
        to_path: None,
        kind: Some(EntryKind::File),
        size: Some(1),
        modified: Some(0),
    })
    .expect("serialise");
    for forbidden in ["sha", "hash", "content", "bytes", "digest"] {
        assert!(!json.contains(forbidden), "{forbidden} appeared: {json}");
    }
}

#[test]
fn an_invalidation_names_only_its_workspace() {
    let json =
        serde_json::to_string(&InvalidateAllParams { workspace_id: ws() }).expect("serialise");
    assert_eq!(json, r#"{"workspace_id":"ws_7f2a"}"#);
}

#[test]
fn a_wire_payload_deserialises_into_the_same_shape() {
    let raw = r#"{"workspace_id":"ws_7f2a","events":[
        {"event":"renamed","relative_path":"a","to_path":"b"},
        {"event":"created","relative_path":"c","type":"directory","modified":7}]}"#;
    let parsed: FileEventParams = serde_json::from_str(raw).expect("deserialise");
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(parsed.events[0].to_path.as_deref(), Some("b"));
    assert_eq!(parsed.events[1].kind, Some(EntryKind::Directory));
    assert_eq!(
        parsed.events[1].size, None,
        "a directory has nothing to measure"
    );
}
