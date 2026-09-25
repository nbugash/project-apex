//! SC-005: a 4 MiB line with no newline in it arrives whole.
//!
//! The case exists because a chunker that flushes on newlines looks correct against every
//! ordinary program and stalls forever here. `fixture_bigline` writes four megabytes with the
//! newline only at the very end, so nothing is delivered at all until the byte bound fires.
//!
//! Two bounds are asserted, and they are different bounds. The **raw** chunk is the value
//! somebody chose, read from the constant the source exports rather than restated, so that
//! changing it moves this test with it. The **frame** is the limit that actually breaks: base64
//! inflates by 4/3, and a raw bound chosen without that in mind produces frames §4.1 refuses.
//!
//! The measurements are printed rather than only compared, per the project's non-functional
//! convention -- a verdict says a bound held, a number says how much room was left.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::output::CHUNK_BYTES;
use apex_engine::application::ports::task_runner::{ResourceLimits, SpawnRequest, TaskRunner};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::Shape;
use apex_protocol::wire::TaskId;
use common::frames::{delivered_bytes, frames_of, Frame, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What `fixture_bigline` writes, before its single trailing newline.
const LINE_BYTES: usize = 4 * 1024 * 1024;

/// §4.1's frame ceiling. The thing the raw bound has to stay under once base64 has inflated it.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

fn run_bigline() -> (Vec<Frame>, Vec<u8>) {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner.clone());

    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_bigline")];
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    // Pipes rather than a terminal: a pty's line discipline would rewrite the trailing newline
    // and, more to the point, this is about the chunker rather than about the device.
    let task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape: Shape::Pipes,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");
    service.adopt(TaskId("big".into()), task.pid, task.control, task.output);

    // Four megabytes through a pipe takes a moment. The wait is on the exit frame, not on a
    // duration -- a sleep long enough to be safe here would be most of a minute.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if frames_of(&sink)
            .iter()
            .any(|f| f.method == "execution/onExit")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    service.close();
    let frames = frames_of(&sink);
    assert!(
        frames.iter().any(|f| f.method == "execution/onExit"),
        "the task never ended; nothing can be concluded about its output"
    );
    let bytes = delivered_bytes(&frames);
    (frames, bytes)
}

#[test]
fn a_four_megabyte_line_arrives_whole_and_within_both_bounds() {
    let (frames, bytes) = run_bigline();

    let output_frames: Vec<&Frame> = frames
        .iter()
        .filter(|f| f.method == "execution/onStdout" || f.method == "execution/onStderr")
        .collect();
    let largest_raw = output_frames
        .iter()
        .filter_map(|f| f.data.as_ref())
        .map(|d| d.len())
        .max()
        .expect("at least one output frame");
    // base64 is 4 bytes out per 3 in, rounded up to a multiple of 4.
    let largest_frame = largest_raw.div_ceil(3) * 4;

    println!("SC-005 total delivered: {} bytes", bytes.len());
    println!("SC-005 output frames: {}", output_frames.len());
    println!(
        "SC-005 largest raw chunk: {largest_raw} bytes (bound {CHUNK_BYTES}, {} spare)",
        CHUNK_BYTES.saturating_sub(largest_raw)
    );
    println!(
        "SC-005 largest encoded frame: {largest_frame} bytes (ceiling {MAX_FRAME_BYTES}, {} spare)",
        MAX_FRAME_BYTES.saturating_sub(largest_frame)
    );

    // Whole. Exactly, not approximately: the 4 MiB line plus its one trailing newline.
    assert_eq!(
        bytes.len(),
        LINE_BYTES + 1,
        "delivered {} bytes, expected {}",
        bytes.len(),
        LINE_BYTES + 1
    );

    // Zero truncation and zero duplication. The fixture writes a repeating a..z, so any byte
    // out of place breaks the sequence at a position this reports.
    let pattern: Vec<u8> = (b'a'..=b'z').collect();
    if let Some(at) = (0..LINE_BYTES).find(|i| bytes[*i] != pattern[i % pattern.len()]) {
        panic!("the pattern breaks at byte {at}: found {:#x}", bytes[at]);
    }
    assert_eq!(bytes[LINE_BYTES], b'\n', "the trailing newline is missing");

    // The chosen bound, read from the source rather than restated.
    assert!(
        largest_raw <= CHUNK_BYTES,
        "a chunk of {largest_raw} bytes exceeds the {CHUNK_BYTES} byte bound"
    );
    // The bound that actually breaks.
    assert!(
        largest_frame <= MAX_FRAME_BYTES,
        "an encoded frame of {largest_frame} bytes exceeds §4.1's {MAX_FRAME_BYTES} byte ceiling"
    );
}

#[test]
fn the_line_is_delivered_in_pieces_rather_than_held_until_it_ends() {
    // The stall this guards against. A chunker waiting for a newline delivers one frame of four
    // megabytes at the very end -- or, past §4.1's ceiling, nothing at all. More than one output
    // frame is what says the byte bound fired while the line was still being written.
    let (frames, _) = run_bigline();
    let output_frames = frames
        .iter()
        .filter(|f| f.method == "execution/onStdout")
        .count();
    assert!(
        output_frames > 1,
        "4 MiB arrived in {output_frames} frame(s); the chunker is waiting for something"
    );
}
