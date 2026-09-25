//! The id-less dispatch path (T036).
//!
//! `dispatch` read `id` with `as_str` and returned `Action::Nothing` **before** the method
//! match, so a frame without an id never reached a handler. `execution/writeStdin` and
//! `execution/resizePty` are the catalogue's first client-to-engine notifications, so nothing
//! in F010's interactive half could have worked. The same line dropped a **numeric** id, which
//! JSON-RPC 2.0 allows, as though it were a notification.
//!
//! **What these tests can and cannot currently discriminate.** Restoring the old `as_str` read
//! fails both numeric-id tests here. It does **not** fail the two notification tests, because
//! `dispatch_notification` has no arms yet: a frame that is dispatched and ignored produces the
//! same `Action::Nothing` as one dropped before the method match. Those two become
//! discriminating when T054 wires a task service through and `execution/writeStdin` has
//! somewhere to land. Until then they assert the contract rather than the routing, which is
//! worth knowing when reading a green run.

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use common::FakeFileSystem;
use std::sync::Arc;

fn dispatch_body(body: &str) -> Action {
    let registry = SessionRegistry::new();
    let fs = Arc::new(FakeFileSystem::new());
    let roots = InMemoryRoots::new(fs.clone());
    let codec = FrameCodec;
    dispatch(&registry, &roots, fs.as_ref(), None, &codec, body)
}

#[test]
fn a_frame_without_an_id_is_a_notification_and_answers_nothing() {
    let action = dispatch_body(
        r#"{"jsonrpc":"2.0","method":"execution/writeStdin","params":{"task_id":"build","data":"aGk="}}"#,
    );
    // Zero bytes on the wire. §4.2: a notification has no response, so producing one would be
    // a frame the client never asked for and cannot match to anything.
    assert!(
        matches!(action, Action::Nothing),
        "a notification must not produce a reply"
    );
}

#[test]
fn an_unknown_notification_is_dropped_in_silence() {
    let action = dispatch_body(r#"{"jsonrpc":"2.0","method":"nonsense/method","params":{}}"#);
    // There is no id to answer and §4.2 leaves nothing else to do. An error frame here would be
    // unmatchable by the client and would break the stream's request/response pairing.
    assert!(matches!(action, Action::Nothing));
}

#[test]
fn a_numeric_id_is_answered_rather_than_dropped() {
    // JSON-RPC 2.0 allows a number. Read with `as_str` this took the notification exit and was
    // silently dropped, so a conforming client using numeric ids would have seen every request
    // hang.
    let action = dispatch_body(r#"{"jsonrpc":"2.0","id":7,"method":"session/ping","params":{}}"#);
    assert!(
        !matches!(action, Action::Nothing),
        "a request with a numeric id must be answered"
    );
}

#[test]
fn a_numeric_id_comes_back_as_a_number() {
    let action = dispatch_body(r#"{"jsonrpc":"2.0","id":7,"method":"session/ping","params":{}}"#);
    let Action::Reply(bytes) = action else {
        panic!("expected a reply");
    };
    let text = String::from_utf8(bytes).expect("utf8");
    // Echoed unchanged. `"id":"7"` is a different id from `"id":7`, and a client matching
    // responses to requests would never pair them.
    assert!(
        text.contains(r#""id":7"#),
        "the id was not echoed as a number: {text}"
    );
    assert!(!text.contains(r#""id":"7""#), "the id was quoted: {text}");
}

#[test]
fn a_string_id_still_comes_back_as_a_string() {
    let action =
        dispatch_body(r#"{"jsonrpc":"2.0","id":"abc","method":"session/ping","params":{}}"#);
    let Action::Reply(bytes) = action else {
        panic!("expected a reply");
    };
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.contains(r#""id":"abc""#), "{text}");
}
