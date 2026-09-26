//! SC-023's engine half: a client that has lost its identities can find them again.
//!
//! Without `execution/list` a task whose identity a client forgot keeps running and is
//! unreachable until the instance idles out -- it cannot be attached to, stopped, or even named.
//! An omitted `workspaceId` therefore enumerates **everything the engine holds**, which is what
//! makes it a recovery path rather than a convenience.
//!
//! It is a **pure read**. Listing touches the task runner zero times: asking each task's control
//! whether it is still running would turn an enumeration into N syscalls, and the answer would
//! still be the engine's own record -- which is what `onExit` updates and what a client would be
//! told either way.
//!
//! Driven against `FakeRunner`, which counts what the port was asked to do, so "zero times" is
//! observed rather than assumed.

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use common::fake_clock::FakeClock;
use common::fake_runner::{FakeRunner, Script};
use common::frames::Sink;
use common::FakeFileSystem;
use std::sync::Arc;

struct Harness {
    service: TaskService,
    runner: Arc<FakeRunner>,
    roots: InMemoryRoots,
    fs: Arc<dyn FileSystem>,
    codec: FrameCodec,
}

impl Harness {
    fn start(&mut self, ws: &str, task: &str, command: &[&str], pty: bool) {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "1", "method": "execution/runTask",
            "params": {
                "workspace_id": ws,
                "task_id": task,
                "command": command,
                "pty": pty
            }
        })
        .to_string();
        let action = self.call(&body);
        let Action::Reply(frame) = action else {
            panic!("a request must be answered");
        };
        let text = String::from_utf8_lossy(&frame).into_owned();
        let reply: serde_json::Value =
            serde_json::from_str(text.split_once("\r\n\r\n").expect("framed").1).expect("json");
        assert!(reply.get("error").is_none(), "start failed: {reply}");
    }

    fn call(&self, body: &str) -> Action {
        dispatch(
            &SessionRegistry::new(),
            &self.roots,
            self.fs.as_ref(),
            None,
            Some(&self.service),
            &self.codec,
            body,
        )
    }

    fn list(&self, workspace: Option<&str>) -> serde_json::Value {
        let params = match workspace {
            Some(ws) => serde_json::json!({ "workspace_id": ws }),
            None => serde_json::json!({}),
        };
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "2", "method": "execution/list", "params": params
        })
        .to_string();
        let Action::Reply(frame) = self.call(&body) else {
            panic!("a request must be answered");
        };
        let text = String::from_utf8_lossy(&frame).into_owned();
        serde_json::from_str(text.split_once("\r\n\r\n").expect("framed").1).expect("json")
    }
}

fn harness() -> Harness {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink)));
    let clock: Arc<FakeClock> = Arc::new(FakeClock::new());
    let runner = Arc::new(FakeRunner::new());
    // Silent and never-ending, so every task stays in the listing for the length of the test.
    for _ in 0..8 {
        runner.script(Script::silent_forever());
    }
    let service = TaskService::new(
        writer,
        Arc::clone(&clock) as Arc<_>,
        Arc::clone(&runner) as Arc<_>,
    );

    let fs_impl = Arc::new(FakeFileSystem::new());
    fs_impl.dir("/a").dir("/b");
    let fs: Arc<dyn FileSystem> = fs_impl.clone();
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("A", "/a").expect("register A");
    roots.register("B", "/b").expect("register B");

    Harness {
        service,
        runner,
        roots,
        fs,
        codec: FrameCodec::new(),
    }
}

fn ids(reply: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = reply["result"]["tasks"]
        .as_array()
        .expect("a tasks array")
        .iter()
        .map(|t| t["task_id"].as_str().unwrap_or_default().to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn an_omitted_workspace_enumerates_every_task_the_engine_holds() {
    let mut h = harness();
    h.start("A", "a1", &["cargo", "build"], true);
    h.start("A", "a2", &["cargo", "test"], false);
    h.start("B", "b1", &["npm", "run", "dev"], true);

    assert_eq!(ids(&h.list(None)), vec!["a1", "a2", "b1"]);
}

#[test]
fn a_named_workspace_lists_only_its_own() {
    let mut h = harness();
    h.start("A", "a1", &["cargo", "build"], true);
    h.start("B", "b1", &["npm", "run", "dev"], true);

    assert_eq!(ids(&h.list(Some("A"))), vec!["a1"]);
    assert_eq!(ids(&h.list(Some("B"))), vec!["b1"]);
}

#[test]
fn a_listing_carries_no_environment() {
    // FR-005a. A listing is exactly "anything that can be read back", and an environment commonly
    // carries credentials. `command` **is** carried, which moves where the accepted boundary
    // sits: a credential in argv becomes readable by any client that enumerates rather than only
    // by the one that started the task. Under A-EC2's single tenancy that is the same developer,
    // so it widens where rather than who -- and it is asserted here so the widening stays a
    // decision rather than becoming an accident.
    let mut h = harness();
    h.start("A", "a1", &["cargo", "build", "--token", "s3cret"], true);

    let reply = h.list(None);
    let rendered = reply.to_string();
    assert!(
        !rendered.contains("\"env\""),
        "the listing carried an environment: {rendered}"
    );
    assert!(
        !rendered.contains("PATH"),
        "the listing carried environment contents: {rendered}"
    );
    // The command is there, deliberately.
    assert!(rendered.contains("cargo"), "{rendered}");
}

#[test]
fn a_listing_says_which_shape_each_task_has() {
    // A client reattaching has to know whether an interrupt is a byte or a signal, and the shape
    // is what decides (A-TASKSTREAM). A listing that omitted it would send the client back for
    // the one thing it needs before it can do anything.
    let mut h = harness();
    h.start("A", "with", &["cargo", "build"], true);
    h.start("A", "without", &["cargo", "test"], false);

    let reply = h.list(None);
    let tasks = reply["result"]["tasks"].as_array().expect("tasks");
    let by_id = |id: &str| {
        tasks
            .iter()
            .find(|t| t["task_id"] == id)
            .unwrap_or_else(|| panic!("{id} is missing: {reply}"))
            .clone()
    };
    assert_eq!(by_id("with")["pty"], true);
    assert_eq!(by_id("without")["pty"], false);
}

#[test]
fn listing_touches_the_runner_zero_times() {
    // The claim that makes it a pure read. A listing that asked each task's control whether it is
    // alive would cost one syscall per task and would still be answering from the engine's record
    // a moment later.
    let mut h = harness();
    h.start("A", "a1", &["cargo", "build"], true);
    h.start("A", "a2", &["cargo", "test"], true);

    let signals_before: usize = (0..2).map(|n| h.runner.control(n).signals().len()).sum();
    let resizes_before: usize = (0..2).map(|n| h.runner.control(n).resizes().len()).sum();

    let _ = h.list(None);
    let _ = h.list(Some("A"));

    let signals_after: usize = (0..2).map(|n| h.runner.control(n).signals().len()).sum();
    let resizes_after: usize = (0..2).map(|n| h.runner.control(n).resizes().len()).sum();
    assert_eq!(signals_before, signals_after, "listing signalled a task");
    assert_eq!(resizes_before, resizes_after, "listing resized a task");
}

#[test]
fn listing_an_unknown_workspace_is_an_empty_list_rather_than_an_error() {
    // A client recovering from lost state may name a workspace the engine has since forgotten.
    // Refusing would deny recovery to exactly the client that needs it; nothing is running there,
    // and saying so is the answer.
    let h = harness();
    let reply = h.list(Some("never-registered"));
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(ids(&reply).len(), 0);
}
