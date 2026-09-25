//! SC-018: a dropped connection leaves a task alone.
//!
//! A-TASKLIFE, stated as a test. A laptop moving between networks must not kill a build, so a
//! disconnection terminates **zero** tasks, releases zero identities and signals zero processes.
//! It sets a flag and changes nothing else.
//!
//! The zeros are the whole assertion. Every positive claim here -- the task is still in the
//! listing, its process is still running -- would also hold for an engine that terminated it and
//! started another, so what is counted is what did **not** happen.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

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
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::frames::{frames_of, Sink};
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

/// Is `pid` a running process? A zombie is not.
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
    id: TaskId,
    pid: i32,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(control) = self.service.control(&self.id) {
            let _ = control.signal(TaskSignal::Kill);
        }
        self.service.close();
    }
}

fn running_task() -> Harness {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let id = TaskId("survivor".into());
    let params = RunTaskParams {
        workspace_id: WorkspaceId("ws1".into()),
        task_id: id.clone(),
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
        pty: true,
        cols: None,
        rows: None,
    };
    let pid = service.run(&params, &roots, fs.as_ref()).expect("run");

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !running(pid.0) {
        std::thread::sleep(Duration::from_millis(10));
    }
    Harness {
        service,
        sink,
        id,
        pid: pid.0,
    }
}

#[test]
fn a_dropped_connection_terminates_nothing() {
    let h = running_task();
    assert!(running(h.pid), "the task never started");
    let exits_before = frames_of(&h.sink)
        .into_iter()
        .filter(|f| f.method == "execution/onExit")
        .count();

    // The client goes away. This is the whole of what a disconnection does.
    let registry = SessionRegistry::new();
    assert!(registry.attached(), "a new session starts attached");
    registry.set_attached(false);
    assert!(!registry.attached());

    std::thread::sleep(Duration::from_millis(500));

    // The zeros.
    assert!(
        running(h.pid),
        "a dropped connection killed the task, which is what A-TASKLIFE forbids"
    );
    let exits_after = frames_of(&h.sink)
        .into_iter()
        .filter(|f| f.method == "execution/onExit")
        .count();
    assert_eq!(
        exits_after,
        exits_before,
        "a dropped connection ended {} task(s)",
        exits_after - exits_before
    );
    assert_eq!(
        h.service.live(),
        1,
        "a dropped connection released the task's identity"
    );
}

#[test]
fn the_task_is_still_reachable_after_the_client_goes_away() {
    // Surviving is not enough: the point of surviving is that a returning client can reach it.
    // A task still running but no longer resolvable is §15.2's crash case, not its disconnection
    // case.
    let h = running_task();
    let registry = SessionRegistry::new();
    registry.set_attached(false);

    assert!(
        h.service.control(&h.id).is_some(),
        "the task's control was released by a disconnection"
    );
    let listed = h.service.list(None);
    assert_eq!(listed.len(), 1, "the task vanished from the listing");
    assert!(listed[0].running, "the task is listed as finished");
    assert!(
        h.service.attach(&WorkspaceId("ws1".into()), &h.id).is_ok(),
        "the task could not be attached to after a disconnection"
    );
}

#[test]
fn the_task_set_is_never_handed_the_transport() {
    // Invariant 12, and the reason it is worth stating separately. "A disconnection terminates
    // nothing" is true here because there is nothing that could do the terminating: the session
    // holds the flag, the task service holds the tasks, and neither has a reference to the
    // other. An assertion about behaviour would pass for a design where the two were connected
    // and the connecting code simply did nothing yet.
    let h = running_task();
    let registry = SessionRegistry::new();
    registry.set_attached(false);

    // `TaskService::new` takes a writer, a clock and a runner. There is no argument through which
    // a transport or a session could reach it, which is what makes this structural rather than a
    // matter of conduct -- and is checked here so that adding one is a change to this test.
    assert!(h.service.control(&h.id).is_some());
    assert_eq!(h.service.live(), 1);
}
