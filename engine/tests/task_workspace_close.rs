//! SC-013: closing a workspace leaves zero of its tasks running, and only its own.
//!
//! The **second workspace is the assertion**, not the setup. A close that stopped everything
//! would satisfy every positive claim about the first workspace's tasks ending, and would kill a
//! build in a window the developer is still working in.
//!
//! Observed through `execution/onExit` and never through a sleep. A-WSCLOSE, as amended, writes
//! the response once every task has been **signalled** rather than ended, because ending takes up
//! to the five-second escalation and the dispatch thread is also the only reader of the client's
//! stdin. So the criterion is checked by waiting for N exits, which are defined events in a
//! defined order -- and the escalations overlap, so the whole workspace costs about five seconds
//! rather than five per task.
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
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::frames::{delivered_bytes, frames_of, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

/// Is `pid` a running process? A zombie is not: `/proc/<pid>` survives until the parent reaps.
fn running(pid: i32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(after) = stat.rsplit_once(')') else {
        return false;
    };
    !matches!(after.1.split_whitespace().next(), Some("Z") | None)
}

struct Harness {
    service: TaskService,
    sink: Sink,
    roots: InMemoryRoots,
    fs: Arc<dyn FileSystem>,
    codec: FrameCodec,
    ids: Vec<TaskId>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for id in &self.ids {
            if let Some(control) = self.service.control(id) {
                let _ = control.signal(TaskSignal::Kill);
            }
        }
        self.service.close();
    }
}

impl Harness {
    fn start(&mut self, ws: &str, task: &str) -> i32 {
        let params = RunTaskParams {
            workspace_id: WorkspaceId(ws.into()),
            task_id: TaskId(task.into()),
            command: vec![fixture("fixture_signals")],
            cwd: None,
            env: Some(
                [(
                    "PATH".to_string(),
                    std::env::var("PATH").unwrap_or_default(),
                )]
                .into_iter()
                .collect(),
            ),
            pty: false,
            cols: None,
            rows: None,
        };
        let pid = self
            .service
            .run(&params, &self.roots, self.fs.as_ref())
            .expect("run");
        self.ids.push(TaskId(task.into()));
        pid.0
    }

    fn close(&self, ws: &str) -> serde_json::Value {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "1", "method": "workspace/close",
            "params": { "workspace_id": ws }
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
        serde_json::from_str(text.split_once("\r\n\r\n").expect("framed").1).expect("json")
    }

    fn exits(&self) -> usize {
        frames_of(&self.sink)
            .into_iter()
            .filter(|f| f.method == "execution/onExit")
            .count()
    }

    fn wait_for_exits(&self, n: usize, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.exits() >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn started(&self) -> usize {
        String::from_utf8_lossy(&delivered_bytes(&frames_of(&self.sink)))
            .matches("READY")
            .count()
    }
}

fn harness() -> Harness {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("A", "/tmp").expect("register A");
    roots.register("B", "/tmp").expect("register B");

    Harness {
        service,
        sink,
        roots,
        fs,
        codec: FrameCodec::new(),
        ids: Vec::new(),
    }
}

#[test]
fn closing_a_workspace_ends_its_tasks_and_leaves_another_workspace_alone() {
    let mut h = harness();
    let a = [h.start("A", "a1"), h.start("A", "a2"), h.start("A", "a3")];
    let b = h.start("B", "b1");

    // All four running before the close. "Zero survivors" is true of tasks that never started.
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && h.started() < 4 {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(h.started(), 4, "not every task started");

    let reply = h.close("A");
    assert!(reply.get("error").is_none(), "the close failed: {reply}");

    // Three exits, waited for rather than slept through. The escalations overlap, so this is
    // about five seconds for the workspace rather than five per task.
    assert!(
        h.wait_for_exits(3, Duration::from_secs(20)),
        "only {} of 3 tasks ended",
        h.exits()
    );

    for (n, pid) in a.iter().enumerate() {
        assert!(!running(*pid), "task a{} survived the close", n + 1);
    }
    // The zero that matters.
    assert!(
        running(b),
        "closing A ended a task of B, which nobody asked to close"
    );
    assert_eq!(h.exits(), 3, "closing A ended {} tasks", h.exits());
}

#[test]
fn a_second_close_is_refused_rather_than_repeated() {
    // -32001, not an idempotent success. A workspace never closes itself, so a second close
    // means the client has lost track of its own state and telling it so is a service. This
    // departs from FR-019, where terminating an already-terminated task succeeds, and the two
    // only look alike: that race is a client racing an end the engine decided (A-WSCLOSE).
    let mut h = harness();
    h.start("A", "a1");
    assert!(h.close("A").get("error").is_none());

    let second = h.close("A");
    assert_eq!(
        second["error"]["code"], -32001,
        "a second close must be -32001, got {second}"
    );
}

#[test]
fn a_workspace_whose_root_has_vanished_still_closes() {
    // -32009 is never returned here. Refusing to stop the tasks of a deleted directory would
    // strand exactly what FR-025 forbids: the tasks are what matter, not the directory.
    let mut h = harness();
    let dir = tempfile::tempdir().expect("tempdir");
    h.roots
        .register("gone", dir.path().to_str().expect("utf8"))
        .expect("register");
    h.start("gone", "g1");
    drop(dir);

    let reply = h.close("gone");
    assert!(
        reply.get("error").is_none(),
        "a workspace whose root is gone must still close, got {reply}"
    );
    assert!(
        h.wait_for_exits(1, Duration::from_secs(20)),
        "the task of a vanished workspace was stranded"
    );
}

#[test]
fn closing_an_unknown_workspace_is_refused() {
    let h = harness();
    let reply = h.close("never-registered");
    assert_eq!(reply["error"]["code"], -32001, "{reply}");
}
