//! SC-015 and US3.7: a command that cannot be started says so, with a code.
//!
//! The codes are the contract and the messages are prose. `-32011` is never `-32003`: §4.4
//! reserves the latter for a path **inside a workspace**, and a missing executable and a missing
//! source file lead to different things being said to the developer. `-32010` is never `-32006`:
//! one means "you already have a task with that identity" and the other means "you have none",
//! and a client choosing its own ids needs to tell them apart.
//!
//! Driven against a real runner, because whether a command can be started is the one question a
//! fake cannot answer for itself.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::TaskSignal;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::TaskId;
use common::frames::{frames_of, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    service: TaskService,
    sink: Sink,
    roots: InMemoryRoots,
    fs: Arc<dyn FileSystem>,
    codec: FrameCodec,
    started: Vec<TaskId>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for id in &self.started {
            if let Some(control) = self.service.control(id) {
                let _ = control.signal(TaskSignal::Kill);
            }
        }
        self.service.close();
    }
}

impl Harness {
    fn run(&mut self, task: &str, command: &[&str]) -> serde_json::Value {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "1", "method": "execution/runTask",
            "params": {
                "workspace_id": "ws1",
                "task_id": task,
                "command": command,
                "pty": false
            }
        })
        .to_string();
        let action = dispatch(
            &SessionRegistry::new(),
            &self.roots,
            self.fs.as_ref(),
            None,
            Some(&self.service),
            &self.codec,
            &body,
        );
        let Action::Reply(frame) = action else {
            panic!("a request must be answered");
        };
        let text = String::from_utf8_lossy(&frame).into_owned();
        let reply: serde_json::Value =
            serde_json::from_str(text.split_once("\r\n\r\n").expect("framed").1).expect("json");
        if reply.get("error").is_none() {
            self.started.push(TaskId(task.into()));
        }
        reply
    }
}

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

fn harness() -> Harness {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    Harness {
        service,
        sink,
        roots,
        fs,
        codec: FrameCodec::new(),
        started: Vec::new(),
    }
}

#[test]
fn a_command_that_does_not_exist_is_refused_with_its_own_code() {
    let mut h = harness();
    let reply = h.run("ghost", &["/nonexistent/definitely-not-a-command"]);

    assert_eq!(
        reply["error"]["code"], -32011,
        "a command that cannot start must be -32011, got {reply}"
    );
    // Never -32003. §4.4 reserves that for a path inside a workspace, and conflating them makes
    // the client say "no such file" about a missing compiler.
    assert_ne!(reply["error"]["code"], -32003, "{reply}");

    // The reason the engine gave, carried rather than dropped: without it the developer has
    // "could not start" and no way to tell a typo from a permission problem.
    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(
        !message.is_empty(),
        "the refusal carried no reason: {reply}"
    );
}

#[test]
fn an_unstartable_command_carries_no_environment() {
    // FR-005a. A refusal is a place a whole environment could be attached for diagnosis, and an
    // environment commonly carries credentials. The command is echoed nowhere either: it is
    // argv, and a credential passed in argv is in it.
    let mut h = harness();
    let reply = h.run(
        "ghost",
        &["/nonexistent/cmd", "--token", "s3cret-should-not-appear"],
    );
    let rendered = reply.to_string();
    assert!(
        !rendered.contains("s3cret-should-not-appear"),
        "the refusal echoed the command's arguments: {rendered}"
    );
    assert!(
        !rendered.contains("PATH="),
        "the refusal carried an environment: {rendered}"
    );
}

#[test]
fn a_failed_start_produces_no_exit_frame() {
    // There was never a task, so there is nothing to have ended. An `onExit` here would give a
    // client an ending for an identity it never successfully started, and FR-023 would then have
    // an identity to release that was never taken.
    let mut h = harness();
    let _ = h.run("ghost", &["/nonexistent/cmd"]);

    std::thread::sleep(Duration::from_millis(300));
    let exits = frames_of(&h.sink)
        .into_iter()
        .filter(|f| f.method == "execution/onExit")
        .count();
    assert_eq!(exits, 0, "a start that failed reported an ending");
}

#[test]
fn an_unstartable_command_and_a_live_identity_are_different_codes() {
    // US3.7, asserted on the **codes**. A message is prose and a code is the contract, and these
    // two lead to opposite corrections: one is "fix your command", the other "you already have
    // one of these running".
    let mut h = harness();

    let unstartable = h.run("a", &["/nonexistent/cmd"]);
    assert_eq!(unstartable["error"]["code"], -32011, "{unstartable}");

    // A real task, then the same identity again.
    let first = h.run("b", &[&fixture("fixture_signals")]);
    assert!(
        first.get("error").is_none(),
        "the first start failed: {first}"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && h.service.control(&TaskId("b".into())).is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let second = h.run("b", &[&fixture("fixture_signals")]);
    assert_eq!(
        second["error"]["code"], -32010,
        "a second task under a live identity must be -32010, got {second}"
    );

    // Neither is -32006, which means the opposite of one of them.
    assert_ne!(unstartable["error"]["code"], -32006, "{unstartable}");
    assert_ne!(second["error"]["code"], -32006, "{second}");
}

/// How many children this process has, running or zombie.
fn children() -> usize {
    let ours = std::process::id();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return false;
            };
            let Some((_, after)) = stat.rsplit_once(')') else {
                return false;
            };
            let mut fields = after.split_whitespace();
            let _state = fields.next();
            fields.next().and_then(|p| p.parse::<u32>().ok()) == Some(ours)
        })
        .count()
}

#[test]
fn a_second_task_under_a_live_identity_never_becomes_a_process() {
    // **FR-031c, counted rather than read.** The refusal returns -32010 either way, so a test
    // that asserts on the reply passes for an implementation that spawns the process and then
    // refuses -- and two builds under one identity is exactly what the refusal exists to prevent.
    //
    // That is not hypothetical. The identity is checked twice: once before the spawn and once by
    // `TaskSet::start` afterwards. Removing the first leaves the reply unchanged and the second
    // process created and reaped, which quickstart §10's fifth mutation does and which every
    // reply-reading assertion passes.
    let mut h = harness();

    let first = h.run("twice", &[&fixture("fixture_signals")]);
    assert!(
        first.get("error").is_none(),
        "the first start failed: {first}"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && h.service.control(&TaskId("twice".into())).is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Let the first settle, so the count below is of a steady state rather than of a spawn in
    // flight.
    std::thread::sleep(Duration::from_millis(200));
    let before = children();

    let second = h.run("twice", &[&fixture("fixture_signals")]);
    assert_eq!(second["error"]["code"], -32010, "{second}");

    // A process spawned and reaped would show here as a zombie or a live child; either is a
    // process that was created.
    std::thread::sleep(Duration::from_millis(200));
    let after = children();
    assert_eq!(
        after,
        before,
        "a refused start created {} process(es); FR-031c says the second must never be created, \
         not created and then cleaned up",
        after.saturating_sub(before)
    );
}
