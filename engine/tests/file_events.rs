//! `workspace/onFileEvent` as it reaches the wire (contracts/file-events.md).
//!
//! Asserted on the serialised frame the watch thread produces, not on the domain types it
//! produces them from. What the client receives is the only thing that matters here, and a
//! translation bug is invisible from the far side of it.

mod common;

use apex_protocol::wire::{
    EntryKind, FileEvent, FileEventKind, FileEventParams, InvalidateAllParams, WorkspaceId,
};

fn rendered(events: Vec<FileEvent>) -> serde_json::Value {
    let params = FileEventParams {
        workspace_id: WorkspaceId("ws1".into()),
        events,
    };
    serde_json::to_value(params).expect("serialise")
}

fn created(path: &str) -> FileEvent {
    FileEvent {
        event: FileEventKind::Created,
        relative_path: path.into(),
        to_path: None,
        kind: Some(EntryKind::File),
        size: Some(42),
        modified: Some(1_758_700_000),
    }
}

#[test]
fn a_batch_is_an_array_and_a_single_event_is_still_a_batch() {
    // One flush is one frame. A burst delivered as hundreds of frames would take the writer
    // hundreds of times ahead of whatever interactive request is behind it (FR-016, §4.6).
    let json = rendered(vec![created("a.rs")]);
    assert!(json["events"].is_array(), "{json}");
    assert_eq!(json["events"].as_array().unwrap().len(), 1);
}

#[test]
fn the_four_kinds_are_the_whole_vocabulary() {
    for (kind, spelling) in [
        (FileEventKind::Created, "created"),
        (FileEventKind::Modified, "modified"),
        (FileEventKind::Deleted, "deleted"),
        (FileEventKind::Renamed, "renamed"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), spelling);
    }
}

#[test]
fn only_a_rename_carries_a_destination() {
    let json = rendered(vec![FileEvent {
        event: FileEventKind::Renamed,
        relative_path: "a.rs".into(),
        to_path: Some("b.rs".into()),
        kind: None,
        size: None,
        modified: None,
    }]);
    assert_eq!(json["events"][0]["to_path"], "b.rs");
    let plain = rendered(vec![created("a.rs")]);
    assert!(plain["events"][0].get("to_path").is_none(), "{plain}");
}

#[test]
fn created_and_modified_carry_the_metadata_a_row_needs() {
    // FR-013a. `files` declares size_bytes, remote_modified_at and is_directory NOT NULL, so
    // without these three a created file cannot be placed in the tree at all.
    let json = rendered(vec![created("src/token.rs")]);
    let e = &json["events"][0];
    assert_eq!(e["type"], "file");
    assert_eq!(e["size"], 42);
    assert_eq!(e["modified"], 1_758_700_000u64);
}

#[test]
fn no_event_carries_bytes_or_a_hash() {
    // FR-013, and structural rather than a rule: there is no field that could hold either.
    let json = rendered(vec![created("a.rs")]).to_string();
    for forbidden in ["sha", "hash", "content", "bytes", "digest"] {
        assert!(
            !json.contains(forbidden),
            "{forbidden} reached the wire: {json}"
        );
    }
}

#[test]
fn an_invalidation_names_only_its_workspace() {
    // It says nothing about which paths moved, because the engine does not know: that is what
    // makes it wholesale, and what makes the client's response lazy re-reading (FR-017).
    let json = serde_json::to_value(InvalidateAllParams {
        workspace_id: WorkspaceId("ws1".into()),
    })
    .expect("serialise");
    assert_eq!(json.as_object().expect("object").len(), 1, "{json}");
    assert_eq!(json["workspace_id"], "ws1");
}

#[test]
fn a_deleted_event_describes_nothing_that_is_no_longer_there() {
    let json = rendered(vec![FileEvent {
        event: FileEventKind::Deleted,
        relative_path: "gone.rs".into(),
        to_path: None,
        kind: None,
        size: None,
        modified: None,
    }]);
    let e = json["events"][0].as_object().expect("object");
    assert_eq!(e.len(), 2, "event and path only: {json}");
}
