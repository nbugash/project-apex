//! SC-029 and FR-005: a task runs as the engine, and as nobody else.
//!
//! The engine does not drop privilege, escalate it, or run a task under another account. A task
//! is a child process of the engine and inherits its credentials, which is what makes §4.7's path
//! containment the whole of the boundary -- if a task could become another user, containing its
//! paths would secure nothing.
//!
//! Read from `/proc/<pid>/status`, because the question is what the **kernel** believes about the
//! process, not what this process intended. Both values are **printed**: a host with a restricted
//! `/proc` cannot answer, and a skip that prints nothing is indistinguishable from a pass.
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

/// The `Uid:` line of a process, as `(real, effective, saved, filesystem)`.
///
/// `None` when `/proc` will not say, which is a real condition on a hardened host and is reported
/// as a skip rather than silently treated as agreement.
fn uids(pid: i32) -> Option<(u32, u32, u32, u32)> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    let mut fields = line
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok());
    Some((
        fields.next()?,
        fields.next()?,
        fields.next()?,
        fields.next()?,
    ))
}

#[test]
fn a_task_runs_as_the_engine_and_as_nobody_else() {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink)));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let id = TaskId("identity".into());
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

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && uids(pid.0).is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }

    let ours = uids(std::process::id() as i32);
    let theirs = uids(pid.0);

    println!("SC-029 engine uid (real, effective, saved, fs): {ours:?}");
    println!("SC-029 task   uid (real, effective, saved, fs): {theirs:?}");

    match (ours, theirs) {
        (Some(engine), Some(task)) => {
            assert_eq!(
                task.1, engine.1,
                "the task's effective uid differs from the engine's"
            );
            // All four, not only the effective one. A saved-set uid that differed would be a
            // process able to become somebody else later, which is the same failure deferred.
            assert_eq!(
                task, engine,
                "the task's credentials differ from the engine's: {task:?} against {engine:?}"
            );
        }
        _ => {
            // Printed above, so a reader sees which side could not be read rather than a silent
            // pass on a host where the question was never asked.
            println!("SC-029 SKIPPED: /proc did not report a Uid line for one of the processes");
        }
    }

    if let Some(control) = service.control(&id) {
        let _ = control.signal(TaskSignal::Kill);
    }
    let mut service = service;
    service.close();
}
