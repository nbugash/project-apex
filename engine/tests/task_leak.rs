//! SC-014: a hundred tasks come and go and leave nothing behind.
//!
//! **A hundred, because one proves nothing.** A leak of one identity or one process per cycle is
//! invisible in a single pass -- the counts differ by one, which reads as a task still finishing
//! -- and unmistakable in a hundred. The number is the measurement, not decoration.
//!
//! Two counts, because they leak for different reasons. An identity is the engine's own record
//! and leaks when nothing releases it after the exit is delivered (FR-023). A child process
//! leaks when nothing reaps it, and a reaped-but-unreleased zombie is a third state again --
//! which is why the process count reads `/proc` rather than trusting the engine's bookkeeping.
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
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::frames::{frames_of, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// SC-014 says a hundred.
const CYCLES: usize = 100;

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

/// How many children this process has, running or zombie.
///
/// Read from `/proc`, because the question is what the kernel still holds and not what the
/// engine believes it released. A zombie counts: it is a process table entry nobody reaped, and
/// a hundred of them is a hundred slots gone.
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
            // ppid is the field after the state, and `comm` may contain spaces and brackets, so
            // the fields are taken after the **last** `)`.
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
fn a_hundred_start_and_exit_cycles_leave_nothing_behind() {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let identities_before = service.live();
    let children_before = children();

    for cycle in 0..CYCLES {
        // The **same** identity every time. A cycle that used a fresh id would leave a hundred
        // distinct records and prove only that a map can grow; reusing one also checks that the
        // release actually happened, because a second start under a live identity is refused.
        let params = RunTaskParams {
            workspace_id: WorkspaceId("ws1".into()),
            task_id: TaskId("cycle".into()),
            command: vec![fixture("fixture_report_env")],
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
        service
            .run(&params, &roots, fs.as_ref())
            .unwrap_or_else(|e| panic!("cycle {cycle} could not start: {e:?}"));

        // Wait for this cycle's ending before starting the next, so the cycles are cycles and
        // not a hundred tasks running at once.
        let want = cycle + 1;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let exits = frames_of(&sink)
                .into_iter()
                .filter(|f| f.method == "execution/onExit")
                .count();
            if exits >= want {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let exits = frames_of(&sink)
            .into_iter()
            .filter(|f| f.method == "execution/onExit")
            .count();
        assert_eq!(exits, want, "cycle {cycle} never ended");
    }

    // A moment for the last release to land, so the count is of what remains rather than of what
    // is still being cleaned up.
    std::thread::sleep(Duration::from_millis(200));
    let identities_after = service.live();
    let children_after = children();

    println!("SC-014 cycles: {CYCLES}");
    println!("SC-014 live identities: {identities_before} before, {identities_after} after");
    println!("SC-014 child processes: {children_before} before, {children_after} after");

    assert_eq!(
        identities_after,
        identities_before,
        "{} identities leaked across {CYCLES} cycles",
        identities_after.saturating_sub(identities_before)
    );
    assert_eq!(
        children_after,
        children_before,
        "{} child processes leaked across {CYCLES} cycles",
        children_after.saturating_sub(children_before)
    );

    service.close();
}
