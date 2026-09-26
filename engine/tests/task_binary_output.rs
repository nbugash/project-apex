//! SC-003: the bytes a task wrote are the bytes the client receives.
//!
//! Driven by a **real process** through the **real** delivery path, because the failure this
//! guards against lives in the encoding and not in the logic: a pipeline that decodes output to
//! text and re-encodes it passes every fake and every ASCII fixture, and silently substitutes
//! U+FFFD for anything that is not valid UTF-8. `fixture_binary` writes four different kinds of
//! invalid sequence so that a decoder which repairs one still fails here.
//!
//! The comparison is a digest rather than an equality on two vectors, because a digest mismatch
//! says "these differ" without printing four megabytes, and because SC-003 is stated as a digest.
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
use apex_engine::domain::task::Shape;
use apex_protocol::wire::TaskId;
use common::frames::{delivered_bytes, frames_of, Sink};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The exact sequence `fixture_binary` writes. Restated here rather than imported, so that a
/// change to one and not the other is a failing test rather than two files agreeing about the
/// wrong thing.
const NOT_UTF8: &[u8] = &[
    0x80, 0xED, 0xA0, 0x80, 0xC0, 0xAF, 0xFF, 0xFE, 0x00, 0x01, 0x7F,
];

/// U+FFFD REPLACEMENT CHARACTER, encoded. Its presence is the symptom SC-003 names.
const REPLACEMENT: &[u8] = &[0xEF, 0xBF, 0xBD];

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Run `fixture_binary` to completion and return every output byte that reached the wire.
fn delivered(shape: Shape) -> Vec<u8> {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner.clone());

    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_binary")];
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    let task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");
    let pid = task.pid;
    service.adopt(TaskId("bin".into()), pid, task.control, task.output);

    // A real clock, so the chunker's time bound elapses on its own. The fixture writes a few
    // dozen bytes and exits, so this waits on the exit rather than on a duration.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if frames_of(&sink)
            .iter()
            .any(|f| f.method == "execution/onExit")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    service.close();
    let frames = frames_of(&sink);
    assert!(
        frames.iter().any(|f| f.method == "execution/onExit"),
        "the task never ended; nothing can be concluded about its output"
    );
    delivered_bytes(&frames)
}

#[test]
fn pipe_output_arrives_with_the_digest_it_was_written_with() {
    // Pipes, not a terminal. A pseudo-terminal has a line discipline that rewrites `\n` as
    // `\r\n` by design, so a byte-exact digest through one would be asserting that the kernel
    // does not do its job. This isolates the engine's own encoding, which is what SC-003 is about.
    let out = delivered(Shape::Pipes);

    let mut expected = Vec::new();
    expected.extend_from_slice(b"BINARY-BEGIN\n");
    expected.extend_from_slice(NOT_UTF8);
    expected.extend_from_slice(b"\nBINARY-END\n");

    assert_eq!(
        digest(&out),
        digest(&expected),
        "delivered {} bytes, expected {}: {out:?}",
        out.len(),
        expected.len()
    );
}

#[test]
fn not_one_byte_is_replaced_on_the_way() {
    // The assertion that catches the bug directly. A lossy decode produces U+FFFD, and a digest
    // mismatch alone would not say *why* -- this does.
    let out = delivered(Shape::Pipes);
    assert_eq!(
        out.windows(REPLACEMENT.len())
            .filter(|w| *w == REPLACEMENT)
            .count(),
        0,
        "U+FFFD appears in delivered output, so something decoded it as text: {out:?}"
    );
    assert!(
        out.windows(NOT_UTF8.len()).any(|w| w == NOT_UTF8),
        "the invalid sequence did not arrive intact: {out:?}"
    );
}

#[test]
fn a_terminal_rewrites_line_endings_and_nothing_else() {
    // Through a pty the framing bytes change, because the line discipline changes them. The
    // invalid sequence contains no `\n`, so it must still arrive verbatim -- which is what says
    // the engine is not the thing doing any rewriting.
    let out = delivered(Shape::Pty { cols: 80, rows: 24 });
    assert!(
        out.windows(NOT_UTF8.len()).any(|w| w == NOT_UTF8),
        "a terminal task lost the invalid sequence: {out:?}"
    );
    assert_eq!(
        out.windows(REPLACEMENT.len())
            .filter(|w| *w == REPLACEMENT)
            .count(),
        0,
        "U+FFFD appears in terminal output: {out:?}"
    );
}
