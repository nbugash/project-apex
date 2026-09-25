//! The fake is tested itself (T040).
//!
//! `fake_fs_selftest.rs` is the precedent. A double that quietly stops honouring its contract
//! does not fail — it makes every test that trusts it pass for the wrong reason, which is worse
//! than a broken double because nothing goes red.
//!
//! Each test here names the guarantee from `contracts/runner-port.md` it stands in for.

mod common;

use apex_engine::application::ports::task_runner::{
    Exit, ReadOutcome, ResourceLimits, SpawnFailure, SpawnRequest, TaskRunner,
};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::{Shape, Stream, TaskSignal};
use common::fake_runner::{FakeRunner, Script, Step};
use common::FakeFileSystem;
use std::sync::Arc;

fn cwd(fs: &Arc<FakeFileSystem>) -> ResolvedPath {
    fs.dir("/w");
    let root = ResolvedPath::canonical_root(std::path::Path::new("/w"), fs.as_ref())
        .expect("canonical root");
    ResolvedPath::resolve(&root, ".", fs.as_ref()).expect("resolve")
}

fn spawn(
    runner: &FakeRunner,
    shape: Shape,
) -> apex_engine::application::ports::task_runner::SpawnedTask {
    let fs = Arc::new(FakeFileSystem::new());
    let dir = cwd(&fs);
    let command = vec!["cargo".to_string(), "test".to_string()];
    let env: Vec<(String, String)> = vec![];
    runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &dir,
            env: &env,
            shape,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn")
}

/// T3: with a pseudo-terminal there is one device, so nothing arrives on stderr.
#[test]
fn a_terminal_task_yields_only_stdout_even_if_the_script_says_otherwise() {
    let runner = FakeRunner::new();
    runner.script(Script::of(vec![Step::Bytes(
        Stream::Stderr,
        b"this was scripted as stderr".to_vec(),
    )]));

    let mut task = spawn(&runner, Shape::Pty { cols: 80, rows: 24 });
    let mut buf = Vec::new();
    let outcome = task.output.read(20, &mut buf);

    // The fake enforces A-TASKSTREAM rather than trusting the script, so a test cannot
    // accidentally assert on a stream a pty task can never produce.
    assert_eq!(
        outcome,
        ReadOutcome::Bytes {
            stream: Stream::Stdout,
            len: 27
        }
    );
}

/// T4 and FR-022: `Ended` comes after the bytes, including bytes produced after the exit.
#[test]
fn ended_comes_after_every_byte_including_those_written_after_the_exit() {
    let runner = FakeRunner::new();
    runner.script(Script::of(vec![
        Step::Bytes(Stream::Stdout, b"before".to_vec()),
        Step::BytesAfterExit(Stream::Stdout, b"after".to_vec()),
    ]));

    let mut task = spawn(&runner, Shape::Pipes);
    let mut buf = Vec::new();

    assert!(matches!(
        task.output.read(20, &mut buf),
        ReadOutcome::Bytes { .. }
    ));
    assert!(matches!(
        task.output.read(20, &mut buf),
        ReadOutcome::Bytes { .. }
    ));
    assert_eq!(task.output.read(20, &mut buf), ReadOutcome::Ended);
    assert_eq!(buf, b"beforeafter");
}

/// T9: the exit vocabulary is open, so a signal this feature never sends is representable.
#[test]
fn a_segfault_is_representable() {
    let runner = FakeRunner::new();
    runner.script(Script::of(vec![]).exiting_with(Exit::Signal(11)));

    let mut task = spawn(&runner, Shape::Pipes);
    let mut buf = Vec::new();
    assert_eq!(task.output.read(20, &mut buf), ReadOutcome::Ended);

    // A closed three-variant Signal would have had nothing to put here, and the shapes that
    // remain are dropping the death or encoding it as Code(139).
    assert_eq!(task.control.reap(), Some(Exit::Signal(11)));
}

/// T13: a signal lands while a read is outstanding.
#[test]
fn a_signal_reaches_a_task_whose_read_is_blocked() {
    let runner = FakeRunner::new();
    runner.script(Script::of(vec![
        Step::Block,
        Step::Bytes(Stream::Stdout, b"released".to_vec()),
    ]));

    let mut task = spawn(&runner, Shape::Pipes);
    let control = Arc::clone(&task.control);
    let observer = runner.control(0);

    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let outcome = task.output.read(20, &mut buf);
        (outcome, buf)
    });

    // The read is blocked. This is the whole reason TaskControl is Sync and separate from
    // TaskOutput: under one trait with &mut self, this call would wait for the read.
    control.signal(TaskSignal::Term).expect("signal");
    assert_eq!(observer.signals(), vec![TaskSignal::Term]);

    observer.release_read();
    let (outcome, buf) = reader.join().expect("join");
    assert!(matches!(outcome, ReadOutcome::Bytes { .. }));
    assert_eq!(buf, b"released");
}

/// T8, SC-012 and SC-027: the signal goes to the group, not the pid.
#[test]
fn a_signal_reaches_every_member_of_the_group() {
    let runner = FakeRunner::new();
    runner.script(Script::of(vec![]).with_group(&[100, 101, 102]));

    let task = spawn(&runner, Shape::Pipes);
    task.control.signal(TaskSignal::Term).expect("signal");

    // A fake recording only that `signal` was called would test half of FR-018: it could not
    // tell an implementation that signals the group from one that signals the pid, which is the
    // difference between SC-012 and SC-027.
    assert_eq!(runner.control(0).signalled_pids(), vec![100, 101, 102]);
}

#[test]
fn every_spawn_failure_is_reachable() {
    for failure in [
        SpawnFailure::NotExecutable,
        SpawnFailure::CwdUnusable,
        SpawnFailure::NoDevice,
        SpawnFailure::LimitRefused,
        SpawnFailure::Failed(std::io::ErrorKind::PermissionDenied),
    ] {
        let runner = FakeRunner::new();
        runner.fail_next(failure);
        let fs = Arc::new(FakeFileSystem::new());
        let dir = cwd(&fs);
        let command = vec!["x".to_string()];
        let env: Vec<(String, String)> = vec![];
        let result = runner.spawn(&SpawnRequest {
            command: &command,
            cwd: &dir,
            env: &env,
            shape: Shape::Pipes,
            limits: ResourceLimits::FIXED,
        });
        assert_eq!(result.err(), Some(failure));
    }
}

#[test]
fn stdin_and_resizes_are_recorded_byte_for_byte() {
    let runner = FakeRunner::new();
    runner.script(Script::of(vec![Step::Idle]));
    let task = spawn(&runner, Shape::Pty { cols: 80, rows: 24 });

    // Bytes, not text: SC-007 asserts they arrive unchanged, and 0x00 and 0xff are the cases a
    // text-assuming path mangles.
    task.control
        .write_stdin(&[b'a', 0x00, 0xff])
        .expect("stdin");
    task.control.resize(120, 40).expect("resize");

    let observer = runner.control(0);
    assert_eq!(observer.stdin(), vec![b'a', 0x00, 0xff]);
    assert_eq!(observer.resizes(), vec![(120, 40)]);
}

#[test]
fn a_four_mib_script_has_no_newline_in_it() {
    let script = Script::four_mib_no_newline();
    let Step::Bytes(_, bytes) = &script.steps[0] else {
        panic!("expected bytes");
    };
    assert_eq!(bytes.len(), 4 * 1024 * 1024);
    // SC-005's case. A chunker flushing on newlines looks correct against every ordinary
    // program and stalls forever here, so the absence is the whole fixture.
    assert!(!bytes.contains(&b'\n'));
}

#[test]
fn a_silent_task_never_ends_on_its_own() {
    let runner = FakeRunner::new();
    runner.script(Script::silent_forever());
    let mut task = spawn(&runner, Shape::Pipes);
    let mut buf = Vec::new();
    for _ in 0..1000 {
        assert_eq!(task.output.read(20, &mut buf), ReadOutcome::Idle);
    }
    assert!(buf.is_empty());
}
