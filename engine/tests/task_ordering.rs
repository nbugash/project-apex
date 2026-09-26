//! SC-004 and SC-011: output arrives in order, and the exit arrives last.
//!
//! Two claims that look like one. SC-004 is about bytes within a stream -- no gaps, no
//! transpositions. SC-011 is about the exit **relative to** those bytes, which is a different
//! failure: an implementation can deliver every byte in order and still let the ending overtake
//! the last of them, leaving a panel that says a build finished above the line where it failed.
//!
//! Run 100 times, because ordering failures are races and a single pass is a coin that came up
//! heads. The fixture writes **immediately before exiting, without flushing and waiting**: a
//! fixture that slept after its last write would hand the delivery path all the slack it needs,
//! and the scenario would pass for an implementation that reorders (quickstart §10, mutation 4).
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
use common::frames::{delivered_bytes, frames_of, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// SC-004 and SC-011 both say "across 100 runs".
const RUNS: usize = 100;

/// What `fixture_ordered` writes: lines numbered 0..LINES, then an immediate exit.
const LINES: usize = 200;

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

/// One run: every output byte, and where the exit landed among the frames.
struct Run {
    bytes: Vec<u8>,
    /// Frame index of the exit, and of the last output frame.
    exit_at: usize,
    last_output_at: usize,
}

fn one_run(shape_pty: bool) -> Run {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let params = RunTaskParams {
        workspace_id: WorkspaceId("ws1".into()),
        task_id: TaskId("ord".into()),
        command: vec![fixture("fixture_ordered")],
        cwd: None,
        env: Some(
            [(
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_default(),
            )]
            .into_iter()
            .collect(),
        ),
        pty: shape_pty,
        cols: None,
        rows: None,
    };
    service.run(&params, &roots, fs.as_ref()).expect("run");

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if frames_of(&sink)
            .iter()
            .any(|f| f.method == "execution/onExit")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    service.close();

    let frames = frames_of(&sink);
    let exit_at = frames
        .iter()
        .position(|f| f.method == "execution/onExit")
        .expect("the task never ended");
    let last_output_at = frames
        .iter()
        .rposition(|f| f.method.starts_with("execution/onStd"))
        .expect("the task produced no output");
    Run {
        bytes: delivered_bytes(&frames),
        exit_at,
        last_output_at,
    }
}

/// The line numbers in `bytes`, in the order they arrived.
fn numbers(bytes: &[u8]) -> Vec<usize> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|l| l.strip_prefix("LINE ")?.trim().parse().ok())
        .collect()
}

#[test]
fn output_reassembles_with_no_gaps_and_no_transpositions() {
    // SC-004. A monotonic sequence is what makes both failures visible: a gap is a missing
    // number and a transposition is a number out of place, and neither is detectable in output
    // that only says "some bytes arrived".
    let run = one_run(false);
    let seen = numbers(&run.bytes);
    assert_eq!(
        seen.len(),
        LINES,
        "expected {LINES} lines, got {}",
        seen.len()
    );
    for (expected, actual) in seen.iter().enumerate() {
        assert_eq!(
            *actual, expected,
            "the sequence breaks at position {expected}: got {actual}"
        );
    }
}

#[test]
fn the_exit_never_overtakes_the_output_it_follows() {
    // SC-011, across 100 runs. Once is a coin that came up heads.
    //
    // Both are asserted per run: the delivered byte count equal to the written one, and the exit
    // after the last output frame. A run that lost bytes and put the exit last would satisfy the
    // ordering claim while failing the thing ordering is for.
    let mut worst_gap = usize::MAX;
    for run in 0..RUNS {
        let result = one_run(false);
        assert_eq!(
            numbers(&result.bytes).len(),
            LINES,
            "run {run} delivered {} of {LINES} lines",
            numbers(&result.bytes).len()
        );
        assert!(
            result.last_output_at < result.exit_at,
            "run {run}: the exit was frame {} and the last output frame {}",
            result.exit_at,
            result.last_output_at
        );
        worst_gap = worst_gap.min(result.exit_at - result.last_output_at);
    }
    println!(
        "SC-011 runs: {RUNS}, smallest margin between last output and exit: {worst_gap} frame(s)"
    );
}

#[test]
fn the_same_holds_for_a_task_with_a_terminal() {
    // A terminal merges the streams onto one device, so there is only one order to get wrong --
    // and the line discipline adds a rewrite between the task and the reader, which is a place
    // an implementation could plausibly reorder.
    let run = one_run(true);
    let seen = numbers(&run.bytes);
    assert_eq!(
        seen.len(),
        LINES,
        "expected {LINES} lines through a terminal"
    );
    for (expected, actual) in seen.iter().enumerate() {
        assert_eq!(
            *actual, expected,
            "the sequence breaks at position {expected}"
        );
    }
    assert!(
        run.last_output_at < run.exit_at,
        "the exit overtook the output"
    );
}
