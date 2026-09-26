//! A task's output reaches the wire, in order, with its exit last (T044, SC-028, FR-022).
//!
//! Driven by `FakeRunner`, so the chunking, the ordering and the exit are exercised against
//! exact inputs rather than against whatever a real process happened to produce.

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::task_runner::{
    Exit, ResourceLimits, SpawnRequest, TaskRunner,
};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::{Shape, Stream};
use apex_protocol::wire::{Pid, TaskId};
use common::fake_clock::FakeClock;
use common::fake_runner::{FakeRunner, Script, Step};
use common::frames::{frames_of, Frame, Sink};
use common::FakeFileSystem;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn run_scripted(script: Script, shape: Shape) -> Vec<Frame> {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let clock: Arc<FakeClock> = Arc::new(FakeClock::new());
    let runner = Arc::new(FakeRunner::new());
    runner.script(script);
    let mut service = TaskService::new(
        writer,
        Arc::clone(&clock) as Arc<_>,
        Arc::clone(&runner) as Arc<_>,
    );

    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    let root = ResolvedPath::canonical_root(std::path::Path::new("/w"), fs.as_ref()).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", fs.as_ref()).expect("cwd");
    let command = vec!["cargo".to_string()];
    let env: Vec<(String, String)> = vec![];
    let task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");

    let id = TaskId("build".into());
    service.adopt(id.clone(), Pid(1), task.control, task.output);

    // The chunker's time bound is driven by the fake clock, so nothing here sleeps for real.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        clock.advance(100);
        if frames_of(&sink)
            .iter()
            .any(|f| f.method == "execution/onExit")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    service.close();
    frames_of(&sink)
}

#[test]
fn a_terminal_task_puts_everything_on_stdout_and_nothing_on_stderr() {
    let frames = run_scripted(
        Script::of(vec![
            Step::Bytes(Stream::Stdout, b"compiling".to_vec()),
            // Scripted as stderr; a pty has one device, so it cannot arrive as one.
            Step::Bytes(Stream::Stderr, b"warning".to_vec()),
        ]),
        Shape::Pty { cols: 80, rows: 24 },
    );

    let stderr_frames = frames
        .iter()
        .filter(|f| f.method == "execution/onStderr")
        .count();
    // SC-028's zero, and it can fail: the script deliberately writes to the error stream.
    assert_eq!(
        stderr_frames, 0,
        "a terminal task produced onStderr: {frames:?}"
    );

    let delivered: Vec<u8> = frames
        .iter()
        .filter(|f| f.method == "execution/onStdout")
        .filter_map(|f| f.data.clone())
        .flatten()
        .collect();
    assert_eq!(delivered, b"compilingwarning");
}

#[test]
fn a_pipes_task_keeps_its_error_stream_separate() {
    let frames = run_scripted(
        Script::of(vec![
            Step::Bytes(Stream::Stdout, b"out".to_vec()),
            Step::Bytes(Stream::Stderr, b"err".to_vec()),
        ]),
        Shape::Pipes,
    );
    assert!(
        frames.iter().any(|f| f.method == "execution/onStderr"),
        "pipes must produce a distinguishable error stream: {frames:?}"
    );
}

#[test]
fn the_exit_arrives_after_every_byte_it_followed() {
    let frames = run_scripted(
        Script::of(vec![
            Step::Bytes(Stream::Stdout, b"the last line of a failing build".to_vec()),
            Step::BytesAfterExit(Stream::Stdout, b" and one more".to_vec()),
        ])
        .exiting_with(Exit::Code(101)),
        Shape::Pipes,
    );

    let exit_at = frames
        .iter()
        .position(|f| f.method == "execution/onExit")
        .expect("an exit was reported");
    let last_output = frames
        .iter()
        .rposition(|f| f.method.starts_with("execution/onStd"))
        .expect("output was delivered");

    // FR-022 and SC-011. Reporting the exit first loses exactly the output a developer needs,
    // to the report of the failure that produced it.
    assert!(
        last_output < exit_at,
        "the exit overtook the output it followed: {frames:?}"
    );
    assert_eq!(frames[exit_at].exit_code, Some(101));
    assert_eq!(frames[exit_at].signal, None);
}

#[test]
fn a_signalled_exit_carries_a_name_rather_than_a_code() {
    let frames = run_scripted(
        Script::of(vec![]).exiting_with(Exit::Signal(11)),
        Shape::Pipes,
    );
    let exit = frames
        .iter()
        .find(|f| f.method == "execution/onExit")
        .expect("an exit was reported");

    // Two distinct states, never 128 + n. A client reading an exit code here would report 139
    // for a segfault, which is the convention Key Entities rejects.
    assert_eq!(exit.signal.as_deref(), Some("SIGSEGV"));
    assert_eq!(exit.exit_code, None);
}
