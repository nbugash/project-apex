//! `workspace/writeFile` on the wire.
//!
//! Asserted against **hand-written JSON** rather than against a round trip of the structs.
//! A round trip is symmetric: rename a field and serialisation and deserialisation rename
//! together, agreeing with each other and with nothing else. The engine and the client share
//! these types, so that symmetry hides exactly the change that would break a third party
//! implementing §4.8 — and A-WIRECASE records what happened when the spelling convention was
//! reasoned about rather than read.

use apex_protocol::wire::{codes, WriteFileParams, WriteFileResult};

#[test]
fn the_params_are_snake_case_on_the_wire() {
    // §4.8's tables are camelCase for readability; the wire carries snake_case, and §4.8 says so
    // three paragraphs below them. A frame written by hand is the only thing that checks it.
    let frame = r#"{
        "workspace_id": "ws1",
        "relative_path": "src/main.rs",
        "content": "fn main() {}",
        "base_sha256": "abc123"
    }"#;
    let parsed: WriteFileParams = serde_json::from_str(frame).expect("the engine must accept this");
    assert_eq!(parsed.workspace_id.0, "ws1");
    assert_eq!(parsed.relative_path, "src/main.rs");
    assert_eq!(parsed.content, "fn main() {}");
    assert_eq!(parsed.base_sha256, "abc123");
}

#[test]
fn camel_case_is_refused_rather_than_silently_defaulted() {
    // The failure this guards is a rename that makes the structs agree with each other and with
    // no specification. If camelCase ever parses, the convention has moved and §4.8 has to move
    // with it -- deliberately, not by accident.
    let frame = r#"{
        "workspaceId": "ws1",
        "relativePath": "src/main.rs",
        "content": "x",
        "baseSha256": "abc"
    }"#;
    assert!(
        serde_json::from_str::<WriteFileParams>(frame).is_err(),
        "camelCase parsed, so the wire spelling has changed without §4.8 changing"
    );
}

#[test]
fn the_result_carries_the_hash_of_what_was_written() {
    let encoded = serde_json::to_string(&WriteFileResult {
        sha256: "deadbeef".into(),
    })
    .expect("encode");
    assert_eq!(encoded, r#"{"sha256":"deadbeef"}"#);
}

#[test]
fn a_frame_missing_the_base_is_refused() {
    // Without a base there is nothing to compare, and defaulting one would turn every save into
    // an unconditional overwrite -- the single failure this method exists to prevent.
    let frame = r#"{"workspace_id":"ws1","relative_path":"a.rs","content":"x"}"#;
    assert!(serde_json::from_str::<WriteFileParams>(frame).is_err());
}

#[test]
fn the_conflict_code_is_the_one_4_4_assigns() {
    // Written as a literal on purpose: the constant and the specification must agree, and a test
    // that read the constant on both sides would agree with itself.
    assert_eq!(codes::WRITE_CONFLICT, -32004);
}

#[test]
fn a_path_escape_and_a_missing_file_are_different_codes() {
    // -32002 is the escape and -32003 is not-found. They were transposed in this feature's own
    // contract until the constants were read; collapsing them would tell a developer their file
    // is missing when the engine refused the path, and the remedies are nothing alike.
    assert_eq!(codes::PATH_REFUSED, -32002);
    assert_eq!(codes::NOT_FOUND, -32003);
    assert_ne!(codes::PATH_REFUSED, codes::NOT_FOUND);
}
