//! SC-007: bytes written to a task's input arrive unchanged.
//!
//! Compared as **bytes**, never as a string. The corruption this guards against is invisible in a
//! string comparison -- a lossy decode repairs the very sequence under test into U+FFFD on both
//! sides, and the two then agree.
//!
//! Pipes rather than a terminal, for the same reason `task_binary_output.rs` uses them: a
//! pseudo-terminal has a line discipline that echoes input, translates `\r`, and turns `0x03`
//! into a signal. All of that is the kernel doing its job, and none of it is the engine passing
//! bytes through unchanged, which is what SC-007 is about. The terminal's own behaviour is
//! `task_signals.rs`'s subject.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::task_runner::{ResourceLimits, SpawnRequest, TaskRunner};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::{Shape, TaskSignal};
use apex_protocol::wire::TaskId;
use common::frames::{delivered_bytes, frames_of, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Control bytes and a sequence that is not valid UTF-8, together.
///
/// `0x0A` and `0x0D` are here deliberately: a pipeline that translated line endings would change
/// them, and with pipes there is no line discipline to blame it on. `0x00` is here because a
/// pipeline built around C strings truncates there and every assertion about earlier bytes passes.
const AWKWARD: &[u8] = &[
    0x01, 0x03, 0x09, 0x0A, 0x0D, 0x1B, 0x7F, 0x00, 0x80, 0xED, 0xA0, 0x80, 0xFF, 0xFE, b'e', b'n',
    b'd',
];

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

/// Write each of `writes` to a running `fixture_echo` and return everything it sent back.
fn echo(writes: &[&[u8]]) -> Vec<u8> {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner.clone());

    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_echo")];
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    let task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape: Shape::Pipes,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");
    let id = TaskId("echo".into());
    service.adopt(id.clone(), task.pid, task.control, task.output);

    let control = service.control(&id).expect("the task is live");
    let expected: usize = writes.iter().map(|w| w.len()).sum();
    for write in writes {
        control.write_stdin(write).expect("write stdin");
    }

    // Wait for the bytes rather than for a duration: the fixture echoes per read and never
    // exits on its own, so there is no exit frame to wait for.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    while Instant::now() < deadline {
        out = delivered_bytes(&frames_of(&sink));
        if out.len() >= expected {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Stop the task before closing the service. `fixture_echo` reads until its stdin reaches
    // EOF, which it never does while this test holds the write end, so its reader thread would
    // still be running -- and `TaskService::close` joins reader threads. Signalling first is how
    // a caller is meant to use it; without this the close waits for a task that is waiting for
    // the close.
    control.signal(TaskSignal::Kill).expect("signal");
    service.close();
    out
}

#[test]
fn control_bytes_and_invalid_utf8_come_back_identical() {
    let out = echo(&[AWKWARD]);
    assert_eq!(
        out, AWKWARD,
        "input changed on the way through: {out:?} != {AWKWARD:?}"
    );
}

#[test]
fn no_line_ending_translation_happens() {
    // A lone `\n`, a lone `\r`, and a `\r\n`, in that order. Any translation collapses or
    // expands one of them, and the total length is the first thing to move.
    let input: &[u8] = b"a\nb\rc\r\nd";
    let out = echo(&[input]);
    assert_eq!(out.len(), input.len(), "a byte was added or removed");
    assert_eq!(out, input);
}

#[test]
fn two_frames_are_two_writes_in_frame_order() {
    // SC-007's ordering half. Each write arrives whole and in the order it was sent, which is
    // what makes typed input mean what the developer typed.
    let out = echo(&[b"first-", b"second-", b"third"]);
    assert_eq!(out, b"first-second-third");
}

#[test]
fn a_write_of_nothing_changes_nothing() {
    // An empty `data` is a legal frame and must not close the stream or insert a byte.
    let out = echo(&[b"a", b"", b"b"]);
    assert_eq!(out, b"ab");
}
