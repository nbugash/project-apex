//! The pty adapter against real processes.
//!
//! Everything that *decides* anything is tested against `FakeRunner` with no process at all.
//! What cannot be faked is whether the mechanism itself works: whether a process handed a
//! pseudo-terminal believes it has one, whether both streams really merge onto the one device,
//! and whether a signal reaches a task's children. Those are kernel behaviours, and a fake
//! asserting them would be asserting our own assumptions back at us.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::task_runner::{
    ReadOutcome, ResourceLimits, SpawnFailure, SpawnRequest, TaskRunner,
};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::{Shape, Stream};
use std::time::{Duration, Instant};

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    // target/debug/deps/<test>-<hash> -> target/debug/examples/<name>
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

fn run(command: Vec<String>, shape: Shape, until_ms: u64) -> (Vec<u8>, Vec<Stream>, bool) {
    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    let runner = PtyRunner::new();
    let mut task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");

    let mut out = Vec::new();
    let mut streams = Vec::new();
    let mut ended = false;
    let deadline = Instant::now() + Duration::from_millis(until_ms);
    while Instant::now() < deadline {
        match task.output.read(20, &mut out) {
            ReadOutcome::Bytes { stream, .. } => streams.push(stream),
            ReadOutcome::Idle => {}
            ReadOutcome::Ended => {
                ended = true;
                break;
            }
            ReadOutcome::Failed(e) => panic!("read failed: {e:?}"),
        }
    }
    (out, streams, ended)
}

#[test]
fn a_task_given_a_terminal_believes_it_has_one() {
    let (out, streams, _) = run(
        vec![fixture("fixture_tty_streams")],
        Shape::Pty { cols: 80, rows: 24 },
        3000,
    );
    let text = String::from_utf8_lossy(&out);

    // FR-002. The process asks the kernel, and the kernel is the only thing that can answer.
    assert!(
        text.contains("isatty(stdout)=true"),
        "a pty task must be told it has a terminal: {text}"
    );
    // SC-028's other half: a terminal is one device, so the loud stderr write arrives on the
    // same descriptor and nothing is tagged Stderr (A-TASKSTREAM).
    assert!(
        text.contains("STDERR-MARKER"),
        "the stderr write should still arrive, merged: {text}"
    );
    assert!(
        streams.iter().all(|s| *s == Stream::Stdout),
        "a terminal task produced a Stderr chunk: {streams:?}"
    );
}

#[test]
fn a_task_given_pipes_keeps_its_streams_apart() {
    let (out, streams, _) = run(vec![fixture("fixture_tty_streams")], Shape::Pipes, 3000);
    let text = String::from_utf8_lossy(&out);

    assert!(
        text.contains("isatty(stdout)=false"),
        "a pipe task must not be told it has a terminal: {text}"
    );
    // The distinction SC-028 measures from the other side: with pipes the two are separable,
    // which is what a caller parsing a build's errors wants.
    assert!(
        streams.contains(&Stream::Stderr),
        "pipes must produce a distinguishable error stream: {streams:?}"
    );
}

#[test]
fn a_terminal_is_created_at_the_size_it_was_asked_for() {
    let (out, _, _) = run(
        vec![fixture("fixture_winsize")],
        Shape::Pty {
            cols: 120,
            rows: 40,
        },
        2000,
    );
    let text = String::from_utf8_lossy(&out);
    // §4.8 defaults an omitted size to 80 x 24 precisely so this is never the kernel's 0 x 0,
    // which is the one value `resizePty` refuses and which a process reads as "no terminal".
    assert!(
        text.contains("SIZE cols=120 rows=40"),
        "the terminal was not created at the requested size: {text}"
    );
}

#[test]
fn the_callers_environment_reaches_the_process() {
    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let env = vec![
        (
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        ),
        ("APEX_FIXTURE_KEY".to_string(), "carried".to_string()),
    ];
    let runner = PtyRunner::new();
    let mut task = runner
        .spawn(&SpawnRequest {
            command: &[fixture("fixture_report_env")],
            cwd: &cwd,
            env: &env,
            shape: Shape::Pipes,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");

    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(3000);
    while Instant::now() < deadline {
        match task.output.read(20, &mut out) {
            ReadOutcome::Ended => break,
            ReadOutcome::Failed(e) => panic!("read failed: {e:?}"),
            _ => {}
        }
    }
    let text = String::from_utf8_lossy(&out);
    // `execvp` takes no environment: with it, this variable would never arrive and the merge
    // the use case performs would be silently discarded. clippy found the unused field; this
    // is what makes its absence observable.
    assert!(
        text.contains("APEX_FIXTURE_KEY carried"),
        "the caller's environment did not reach the process: {text}"
    );
    assert!(text.contains("PATH set"), "PATH did not survive: {text}");
}

#[test]
fn a_command_that_cannot_be_started_is_refused_rather_than_becoming_a_task() {
    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let env: Vec<(String, String)> = vec![];
    let runner = PtyRunner::new();
    let result = runner.spawn(&SpawnRequest {
        command: &["/definitely/not/a/program".to_string()],
        cwd: &cwd,
        env: &env,
        shape: Shape::Pipes,
        limits: ResourceLimits::FIXED,
    });

    // FR-004 and SC-015 live entirely in the failure path. The spawn itself succeeds at the
    // fork -- the failure is the child's -- so this asserts the shape the use case sees.
    match result {
        Ok(task) => {
            // The fork succeeded and the exec did not; the child exits 127. That is still a
            // command that could not be started, and the use case must not report it as a task
            // that ran.
            let mut task = task;
            let mut out = Vec::new();
            let deadline = Instant::now() + Duration::from_millis(2000);
            while Instant::now() < deadline {
                if matches!(task.output.read(20, &mut out), ReadOutcome::Ended) {
                    break;
                }
            }
            assert_eq!(
                task.control.reap(),
                Some(apex_engine::application::ports::task_runner::Exit::Code(
                    127
                )),
                "a failed exec must surface as 127, which is what makes it -32011"
            );
        }
        Err(e) => assert_eq!(e, SpawnFailure::NotExecutable),
    }
}
