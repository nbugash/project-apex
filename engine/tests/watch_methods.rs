//! `workspace/watch` and `workspace/unwatch` over the dispatch boundary
//! (contracts/watch-methods.md).

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::watchers::{WatcherFactory, Watchers};
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use common::fake_clock::FakeClock;
use common::fake_watcher::FakeWatcher;
use common::FakeFileSystem;
use std::sync::Arc;

struct Harness {
    registry: SessionRegistry,
    roots: InMemoryRoots,
    fs: Arc<FakeFileSystem>,
    watchers: Watchers,
    codec: FrameCodec,
}

fn harness(capacity: Option<usize>) -> Harness {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/ws").dir("/ws/src").dir("/ws/node_modules");
    fs.file("/ws/src/main.rs", b"fn main() {}");

    let shared: Arc<dyn FileSystem> = fs.clone();
    let roots = InMemoryRoots::new(Arc::clone(&shared));
    roots.register("ws1", "/ws").expect("register");

    let factory: WatcherFactory = Box::new(move |_root| {
        let watcher = match capacity {
            Some(n) => FakeWatcher::with_capacity(n),
            None => FakeWatcher::new(),
        };
        Some((
            Box::new(watcher) as Box<_>,
            Arc::new(FakeClock::new()) as Arc<_>,
        ))
    });
    let codec = FrameCodec::new();
    Harness {
        registry: SessionRegistry::new(),
        roots,
        fs,
        watchers: Watchers::new(
            factory,
            Arc::clone(&shared),
            Arc::new(FrameWriter::new(Box::new(std::io::sink()))),
            codec.clone(),
        ),
        codec,
    }
}

fn call(h: &Harness, method: &str, params: serde_json::Value) -> serde_json::Value {
    let body = serde_json::json!({"jsonrpc":"2.0","id":"1","method":method,"params":params});
    let action = dispatch(
        &h.registry,
        &h.roots,
        h.fs.as_ref() as &dyn FileSystem,
        Some(&h.watchers),
        &h.codec,
        &body.to_string(),
    );
    let Action::Reply(frame) = action else {
        panic!("a request must be answered");
    };
    let text = String::from_utf8_lossy(&frame).into_owned();
    let body = text
        .split_once("\r\n\r\n")
        .expect("a framed reply")
        .1
        .to_string();
    serde_json::from_str(&body).expect("json")
}

#[test]
fn watching_returns_a_count_and_an_empty_refusal_list() {
    let h = harness(None);
    let reply = call(
        &h,
        "workspace/watch",
        serde_json::json!({
            "workspace_id": "ws1", "paths": ["src"]
        }),
    );
    let result = &reply["result"];
    assert!(result["watching"].as_u64().unwrap() >= 1, "{reply}");
    // Absence must be a positive statement: a client that cannot tell "nothing was refused"
    // from "the field is missing" cannot satisfy FR-005.
    assert_eq!(
        result["refused"]
            .as_array()
            .expect("refused is present")
            .len(),
        0
    );
}

#[test]
fn watching_the_same_path_twice_changes_nothing() {
    let h = harness(None);
    let first = call(
        &h,
        "workspace/watch",
        serde_json::json!({
            "workspace_id": "ws1", "paths": ["src"]
        }),
    );
    let second = call(
        &h,
        "workspace/watch",
        serde_json::json!({
            "workspace_id": "ws1", "paths": ["src"]
        }),
    );
    assert_eq!(
        first["result"]["watching"], second["result"]["watching"],
        "idempotent"
    );
}

#[test]
fn an_excluded_path_comes_back_as_a_refusal_with_a_reason() {
    let h = harness(None);
    let reply = call(
        &h,
        "workspace/watch",
        serde_json::json!({
            "workspace_id": "ws1", "paths": ["node_modules"]
        }),
    );
    let refused = reply["result"]["refused"].as_array().expect("refused");
    assert_eq!(refused.len(), 1, "{reply}");
    assert_eq!(refused[0]["reason"], "excluded");
    assert_eq!(refused[0]["path"], "node_modules");
}

#[test]
fn exhausted_capacity_is_a_refusal_and_not_an_error() {
    // FR-005a. The call succeeds; the workspace stays open and browsable.
    let h = harness(Some(0));
    let reply = call(
        &h,
        "workspace/watch",
        serde_json::json!({
            "workspace_id": "ws1", "paths": ["src"]
        }),
    );
    assert!(
        reply.get("error").is_none(),
        "a refusal is data, not an error: {reply}"
    );
    assert_eq!(reply["result"]["refused"][0]["reason"], "capacity");
}

#[test]
fn an_unregistered_workspace_is_told_to_register() {
    let h = harness(None);
    let reply = call(
        &h,
        "workspace/watch",
        serde_json::json!({
            "workspace_id": "never", "paths": ["src"]
        }),
    );
    assert_eq!(reply["error"]["code"], -32001, "{reply}");
}

#[test]
fn unwatching_returns_only_the_count() {
    let h = harness(None);
    call(
        &h,
        "workspace/watch",
        serde_json::json!({"workspace_id":"ws1","paths":["src"]}),
    );
    let reply = call(
        &h,
        "workspace/unwatch",
        serde_json::json!({
            "workspace_id": "ws1", "paths": ["src"]
        }),
    );
    assert!(reply["result"]["watching"].is_number(), "{reply}");
    assert!(
        reply["result"].get("refused").is_none(),
        "unwatch refuses nothing"
    );
}

#[test]
fn unwatching_something_never_watched_is_not_an_error() {
    let h = harness(None);
    let reply = call(
        &h,
        "workspace/unwatch",
        serde_json::json!({
            "workspace_id": "ws1", "paths": ["src"]
        }),
    );
    assert!(reply.get("error").is_none(), "{reply}");
}

#[test]
fn a_build_that_cannot_watch_says_so_rather_than_appearing_to() {
    // FR-027 and A-WATCHLOCAL: the workspace stays usable and the loss is stated.
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/ws");
    let shared: Arc<dyn FileSystem> = fs.clone();
    let roots = InMemoryRoots::new(Arc::clone(&shared));
    roots.register("ws1", "/ws").expect("register");
    let codec = FrameCodec::new();
    let body = serde_json::json!({
        "jsonrpc":"2.0","id":"1","method":"workspace/watch",
        "params":{"workspace_id":"ws1","paths":["."]}
    });
    let action = dispatch(
        &SessionRegistry::new(),
        &roots,
        fs.as_ref() as &dyn FileSystem,
        None,
        &codec,
        &body.to_string(),
    );
    let Action::Reply(frame) = action else {
        panic!("answered")
    };
    let text = String::from_utf8_lossy(&frame).into_owned();
    assert!(text.contains("cannot watch"), "{text}");
}
