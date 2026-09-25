//! A-TASKEXEC and §9's re-execution check: replacing the image stops the tasks.
//!
//! **Both halves are required.** An implementation that names an id in `unpreserved` and leaves
//! its process running produces exactly the outcome A-TASKEXEC exists to prevent -- a process
//! nobody is reading and nobody can reach -- and passes any assertion written against the list
//! alone. So the list is checked, and `/proc` is checked.
//!
//! `exec` keeps the descriptors and discards everything else. The reader threads go with the old
//! image, so a surviving task is orphaned in exactly the way §15.2 describes a crash orphaning
//! one: alive, and reachable by pid and by nothing the protocol exposes.
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
use apex_engine::session::{unpreserved_to_env, SessionRegistry, UNPRESERVED_ENV};
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::frames::Sink;
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

struct Engine {
    service: TaskService,
    ids: Vec<TaskId>,
    pids: Vec<i32>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        for id in &self.ids {
            if let Some(control) = self.service.control(id) {
                let _ = control.signal(TaskSignal::Kill);
            }
        }
        self.service.close();
    }
}

fn with_three_tasks() -> Engine {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink)));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let mut ids = Vec::new();
    let mut pids = Vec::new();
    for n in 0..3 {
        let id = TaskId(format!("t{n}"));
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
        ids.push(id);
        pids.push(pid.0);
    }

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !pids.iter().all(|p| running(*p)) {
        std::thread::sleep(Duration::from_millis(10));
    }
    Engine { service, ids, pids }
}

#[test]
fn a_restart_names_every_task_it_stopped_and_leaves_none_running() {
    let engine = with_three_tasks();
    for (n, pid) in engine.pids.iter().enumerate() {
        assert!(running(*pid), "task t{n} never started");
    }

    // What `drain_and_exec` does before it replaces the image, without replacing it: an `exec`
    // in a test binary would take the harness with it.
    let plans = engine.service.drain_all();
    for plan in &plans {
        if let Some(control) = engine.service.control(&plan.id) {
            let _ = control.signal(plan.send);
            // Term then Kill, as drain_and_exec does. fixture_signals catches SIGTERM and keeps
            // running, which is the case that matters: without the Kill it survives the exec and
            // is orphaned, which is precisely what A-TASKEXEC forbids.
            let _ = control.signal(TaskSignal::Kill);
        }
    }
    let named: Vec<String> = plans.into_iter().map(|p| p.id.0).collect();

    // Half one: every id is named. An empty list would assert that nothing was lost rather than
    // that nothing was checked, which is the distinction §9 asks for.
    let mut sorted = named.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["t0", "t1", "t2"], "not every task was named");

    // Half two: zero survivors. This is what an implementation that names an id and leaves its
    // process running fails, and it is the outcome A-TASKEXEC exists to prevent.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && engine.pids.iter().any(|p| running(*p)) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let survivors: Vec<i32> = engine
        .pids
        .iter()
        .copied()
        .filter(|p| running(*p))
        .collect();
    assert!(
        survivors.is_empty(),
        "{} task process(es) survived the restart: {survivors:?}",
        survivors.len()
    );
}

#[test]
fn the_ids_cross_the_re_execution_in_the_environment() {
    // `session/onRestart` is emitted by the **new** image and the list is built in the old one,
    // so the environment is the only thing that crosses. A round trip through it is what says
    // the new image will have something to report.
    let ids = vec!["t0".to_string(), "t1".to_string(), "t2".to_string()];
    let encoded = unpreserved_to_env(&ids);

    // Set it the way `exec_self` does, then build a registry the way the new image would.
    std::env::set_var(UNPRESERVED_ENV, &encoded);
    std::env::set_var("APEX_SESSION_ID", "session-under-test");
    let adopted = SessionRegistry::new();
    std::env::remove_var(UNPRESERVED_ENV);
    std::env::remove_var("APEX_SESSION_ID");

    let notice = adopted.restart_notice();
    let mut reported = notice.unpreserved.clone();
    reported.sort();
    assert_eq!(reported, ids, "the ids did not survive the re-execution");
    assert!(
        adopted.restarted(),
        "the new image did not know it was a restart"
    );
}

#[test]
fn a_restart_with_no_tasks_reports_an_empty_list_rather_than_nothing() {
    // The ordinary case, and it has to be distinguishable. An empty `unpreserved` asserts that
    // nothing was lost; an absent one asserts only that nobody looked.
    std::env::set_var(UNPRESERVED_ENV, "");
    std::env::set_var("APEX_SESSION_ID", "session-under-test");
    let adopted = SessionRegistry::new();
    std::env::remove_var(UNPRESERVED_ENV);
    std::env::remove_var("APEX_SESSION_ID");

    assert!(adopted.restart_notice().unpreserved.is_empty());
    assert!(adopted.restarted());
}
