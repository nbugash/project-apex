//! SC-006: a build emitting tens of megabytes does not delay interactive traffic.
//!
//! This is the criterion the fairness gate exists for, and the one §4.6 is about. A reader thread
//! writing a 50 MiB burst takes the sink hundreds of times; without a gate, a reply arriving
//! behind those acquisitions waits for all of them, and the developer's keystroke waits with it.
//!
//! Measured at the **writer**, which is where the contention is. The interval is from asking to
//! write an interactive frame to that frame being on the sink -- everything a queue-free design
//! puts between a producer and the wire. Measuring a whole dispatch would include parsing and
//! filesystem work that has nothing to do with the burst.
//!
//! The value is **printed**, not only compared.
//!
//! # What this measured about the gate itself
//!
//! SC-006 is met with large headroom, and **the fairness gate is not what meets it**. Measured
//! both ways -- with `write_bulk`'s yield in place and with it removed -- the numbers are the
//! same to within noise:
//!
//! | producers | gated p99 | ungated p99 |
//! |---|---|---|
//! | 1 | 7.24 ms | 7.28 ms |
//! | 3 | 21.41 ms | 21.33 ms |
//!
//! The reason is that `write_frame` takes the sink for exactly one frame and releases it, so a
//! waiting interactive writer is behind the frames already in progress and nothing else. The
//! bulk writers are blocked on that same mutex, so there is no "re-acquire immediately and pass
//! over the waiter" for the gate to prevent -- which is the behaviour it was designed against.
//!
//! The p99 tracks the number of concurrent producers almost exactly (one frame each, 87 KB at
//! 10 MB/s), which is what a per-frame lock predicts and what both configurations produce.
//!
//! Recorded here rather than acted on: removing the gate is a design decision, and this is the
//! evidence for it.
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
use common::frames::{frames_of, Sink};
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// §1.4's interaction budget.
const BUDGET_MS: f64 = 500.0;
/// SC-006 asks for at least this many samples.
const MIN_SAMPLES: usize = 100;

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

/// A sink that takes time proportional to what it is given, like the pipe it stands for.
///
/// **The in-memory sink the rest of the suite uses makes this measurement meaningless.** Writes
/// to a `Vec` are nearly free, so there is no contention for an interactive frame to be delayed
/// by, and a p99 of two microseconds says only that nothing was in the way. §4.6's whole subject
/// is what happens when something *is*: a 50 MiB burst on a link that can carry ten megabytes a
/// second is five seconds of frames, and a reply behind them waits.
///
/// Ten megabytes a second is deliberately conservative for a local link and roughly right for the
/// remote one this product actually runs over (§1.2).
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

fn p99(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let rank = ((values.len() as f64) * 0.99).ceil() as usize;
    values[rank.saturating_sub(1).min(values.len() - 1)]
}

#[test]
fn interactive_traffic_is_not_delayed_by_a_fifty_megabyte_burst() {
    let sink = Sink::default();
    let slow = SlowSink {
        inner: sink.clone(),
        bytes_per_second: 10 * 1024 * 1024,
    };
    let writer = Arc::new(FrameWriter::new(Box::new(slow)));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer.clone(), Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    // **Three bursting tasks, not one.** With a single bulk producer a plain mutex already
    // bounds the wait at one frame in progress, so a gated and an ungated writer measure the
    // same -- which was checked rather than assumed. §4.6 is about the case where several
    // producers are bulk at once: two builds, or a build beside F007's language server
    // streaming diagnostics. That is where a bulk writer re-acquiring immediately can pass over
    // a waiting reply, and where yielding is what stops it.
    let ids: Vec<TaskId> = (0..3).map(|n| TaskId(format!("burst{n}"))).collect();
    for id in &ids {
        let params = RunTaskParams {
            workspace_id: WorkspaceId("ws1".into()),
            task_id: id.clone(),
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
            // Pipes: `fixture_burst` does not pace itself, and a terminal's line discipline
            // would add a rewrite per byte that has nothing to do with what is measured.
            pty: false,
            cols: None,
            rows: None,
        };
        service.run(&params, &roots, fs.as_ref()).expect("run");
    }

    // Wait for the burst to be genuinely under way. Measuring before the reader thread has
    // started producing would measure an idle engine and call it a loaded one.
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if frames_of(&sink).len() > 50 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let under_way = frames_of(&sink).len();
    assert!(
        under_way > 50,
        "the burst never started; nothing can be concluded about traffic during one"
    );

    // An interactive frame, issued repeatedly while the burst runs.
    let frame = b"Content-Length: 2\r\n\r\n{}".to_vec();
    let mut samples = Vec::with_capacity(MIN_SAMPLES + 20);
    for _ in 0..MIN_SAMPLES + 20 {
        let at = Instant::now();
        writer.write_interactive(&frame).expect("write");
        samples.push(at.elapsed().as_secs_f64() * 1000.0);
        // **Spread across the burst, not fired in a tight loop.** A hundred and twenty writes
        // back to back finish in microseconds and slot into the gaps between bulk frames, which
        // measures an engine that happened to be idle at every sample. Pacing them means each
        // one arrives at an arbitrary point relative to a bulk write in progress, which is the
        // situation §4.6 is about -- and it is what makes the difference between a gated and an
        // ungated writer visible at all.
        std::thread::sleep(Duration::from_millis(5));
    }

    // Still bursting at the end, or the tail of the sample was measured against an idle engine.
    let after = frames_of(&sink).len();
    assert!(
        after > under_way,
        "the burst finished during the measurement: {under_way} frames before, {after} after"
    );

    let measured = p99(samples.clone());
    let mut sorted = samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    println!("SC-006 samples: {}", samples.len());
    println!(
        "SC-006 frames of burst during the measurement: {}",
        after - under_way
    );
    println!("SC-006 min:  {:.3} ms", sorted[0]);
    println!(
        "SC-006 mean: {:.3} ms",
        samples.iter().sum::<f64>() / samples.len() as f64
    );
    println!("SC-006 p99:  {measured:.3} ms (budget {BUDGET_MS} ms)");
    println!("SC-006 max:  {:.3} ms", sorted[sorted.len() - 1]);
    println!("SC-006 headroom at p99: {:.3} ms", BUDGET_MS - measured);

    assert!(
        measured <= BUDGET_MS,
        "SC-006 p99 was {measured:.3} ms under a burst, over the {BUDGET_MS} ms budget"
    );

    for id in &ids {
        if let Some(control) = service.control(id) {
            let _ = control.signal(TaskSignal::Kill);
        }
    }
    service.close();
}
