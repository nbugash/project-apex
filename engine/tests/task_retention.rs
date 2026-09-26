//! SC-021: every byte a task writes is delivered, and the engine holds few of them.
//!
//! Two claims, and the second is the one with a number. Nothing is dropped -- FR-013 slows a
//! producer and never truncates its output -- and the **retention buffer**, which since
//! [CONFLICT 9] is the only place the engine holds a task's bytes at all, stays at or under the
//! 4 MiB the source exports.
//!
//! Driven with a **blocking** sink. With a free sink the reader never waits, so nothing is ever
//! held and the bound is satisfied by never being approached, which measures nothing. A sink that
//! takes time makes the reader wait inside `emit_chunk` with a chunk retained, which is the state
//! the bound is about -- and it makes the backpressure visible: a reader that is not reading is a
//! pseudo-terminal buffer filling, and then a task blocked in `write`.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::output::{CHUNK_BYTES, RETENTION_BYTES};
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::TaskSignal;
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::frames::{delivered_bytes, frames_of, Sink};
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What `fixture_bigline` writes, before its single trailing newline.
const LINE_BYTES: usize = 4 * 1024 * 1024;

/// A sink that takes time, so the reader waits with a chunk retained.
#[derive(Clone)]
struct SlowSink {
    inner: Sink,
    bytes_per_second: usize,
}

impl Write for SlowSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let nanos = (buf.len() as u64 * 1_000_000_000) / self.bytes_per_second as u64;
        std::thread::sleep(Duration::from_nanos(nanos));
        self.inner.clone().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

#[test]
fn every_byte_is_delivered_and_the_engine_holds_few_of_them() {
    let sink = Sink::default();
    let slow = SlowSink {
        inner: sink.clone(),
        bytes_per_second: 8 * 1024 * 1024,
    };
    let writer = Arc::new(FrameWriter::new(Box::new(slow)));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let id = TaskId("hold".into());
    let params = RunTaskParams {
        workspace_id: WorkspaceId("ws1".into()),
        task_id: id.clone(),
        command: vec![fixture("fixture_bigline")],
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
    service.run(&params, &roots, fs.as_ref()).expect("run");

    // Sample what the engine is holding **while** it is holding it. Sampled after the task ends
    // the answer is always zero, which is true and says nothing about the bound.
    let peak = Arc::new(AtomicUsize::new(0));
    let sampling = {
        let peak = Arc::clone(&peak);
        let sink = sink.clone();
        let id = id.clone();
        let service: &TaskService = &service;
        // Borrowed into a scoped thread below; the loop ends when the exit reaches the wire.
        move || {
            let deadline = Instant::now() + Duration::from_secs(60);
            while Instant::now() < deadline {
                if let Some(held) = service.retained_bytes(&id) {
                    peak.fetch_max(held, Ordering::Relaxed);
                }
                if frames_of(&sink)
                    .iter()
                    .any(|f| f.method == "execution/onExit")
                {
                    return;
                }
                std::thread::yield_now();
            }
        }
    };
    std::thread::scope(|scope| {
        scope.spawn(sampling);
    });

    let frames = frames_of(&sink);
    assert!(
        frames.iter().any(|f| f.method == "execution/onExit"),
        "the task never ended"
    );
    let delivered = delivered_bytes(&frames);
    let held = peak.load(Ordering::Relaxed);

    println!("SC-021 bytes written:   {}", LINE_BYTES + 1);
    println!("SC-021 bytes delivered: {}", delivered.len());
    println!(
        "SC-021 bytes held (peak): {held} (bound {RETENTION_BYTES}, {} spare)",
        RETENTION_BYTES.saturating_sub(held)
    );

    // Nothing dropped. FR-013 slows a producer; it never truncates.
    assert_eq!(
        delivered.len(),
        LINE_BYTES + 1,
        "{} of {} bytes were delivered",
        delivered.len(),
        LINE_BYTES + 1
    );
    // Read from the constant, never restated: a test with 4194304 in it agrees with a number
    // rather than with the decision the number came from.
    assert!(
        held <= RETENTION_BYTES,
        "the engine held {held} bytes, past the {RETENTION_BYTES} byte bound"
    );

    if let Some(control) = service.control(&id) {
        let _ = control.signal(TaskSignal::Kill);
    }
    service.close();
}

#[test]
fn a_detached_engine_holds_the_bound_and_stops_reading() {
    // **FR-013a, and the only state in which the bound is reached at all.** While a client is
    // attached, retention is released as each frame goes out, so what is held is whatever is in
    // flight -- one chunk -- and an assertion made there is satisfied by never approaching the
    // ceiling. That was this file's first version, and the mutation that lets the reader read
    // past the bound passed against it.
    //
    // Detached, nothing drains. Output accumulates until the bound, and then the reader stops
    // calling `read`: the pseudo-terminal's buffer fills and the task blocks in its own `write`.
    // Nothing is dropped, nothing is marked and nothing is announced.
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    // Detached **before** the task starts, so nothing it writes is ever drained.
    service.detach();

    let id = TaskId("held".into());
    let params = RunTaskParams {
        workspace_id: WorkspaceId("ws1".into()),
        task_id: id.clone(),
        // `fixture_burst` writes 50 MiB as fast as the pipe takes it, which is far past the
        // 4 MiB bound -- a fixture that fitted inside it would never trigger the mechanism.
        command: vec![fixture("fixture_burst")],
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
    service.run(&params, &roots, fs.as_ref()).expect("run");

    // Long enough for a 50 MiB producer to have run well past the bound had nothing stopped it.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut peak = 0usize;
    while Instant::now() < deadline {
        peak = peak.max(service.retained_bytes(&id).unwrap_or(0));
        std::thread::sleep(Duration::from_millis(5));
    }

    println!("SC-021 detached: bytes held {peak} (bound {RETENTION_BYTES})");
    println!(
        "SC-021 detached: bytes on the wire {} (a detached engine writes to nobody)",
        delivered_bytes(&frames_of(&sink)).len()
    );

    // The bound is **reached**, or this measures a producer that finished early rather than a
    // reader that stopped.
    assert!(
        peak > 0,
        "nothing was retained, so the detached path was never exercised"
    );
    // And not exceeded by more than the chunk in flight when the bound was crossed.
    assert!(
        peak <= RETENTION_BYTES + CHUNK_BYTES,
        "the engine held {peak} bytes, past the {RETENTION_BYTES} byte bound"
    );
    // Nothing went out: there is nobody to write to.
    assert_eq!(
        delivered_bytes(&frames_of(&sink)).len(),
        0,
        "a detached engine wrote to the wire"
    );

    if let Some(control) = service.control(&id) {
        let _ = control.signal(TaskSignal::Kill);
    }
    service.reattach();
    service.close();
}
