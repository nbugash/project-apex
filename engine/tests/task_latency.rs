//! SC-001: from a task writing a byte to that byte reaching the client, p99 under 500 ms.
//!
//! Measured as **one interval** across a locally spawned engine, deliberately. Summing an
//! engine-side p99 and a client-side p99 does not give a p99 of the whole -- it gives a bound on
//! the 98th percentile, because the two tails can coincide. If that composition is ever used it
//! must be reported as p98 and said so.
//!
//! The interval starts at the fixture's own `CLOCK_MONOTONIC` stamp, not at the spawn: everything
//! between exec and the first write belongs to the loader, and counting it would report the cost
//! of starting a program as the cost of delivering its output. It ends when the frame is handed
//! to the writer's sink, which is the last point inside this process before the bytes are a
//! client's problem -- so the harness's own parsing and assertion time is excluded rather than
//! measured.
//!
//! The value is **printed**, not only compared. A verdict says a bound held; a number says how
//! much room was left, which is the thing that tells you whether it is about to stop holding.
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
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// SC-001's bound.
const BUDGET_MS: f64 = 500.0;

/// SC-001 asks for at least this many samples. The fixture writes more, and the surplus absorbs
/// the first few writes, whose page-fault cost is real but is not what the criterion measures.
const MIN_SAMPLES: usize = 100;

fn monotonic_nanos() -> u128 {
    // SAFETY: `ts` is a valid, fully-owned `timespec`, and `clock_gettime` only writes into it.
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    assert_eq!(rc, 0, "clock_gettime failed");
    ts.tv_sec as u128 * 1_000_000_000 + ts.tv_nsec as u128
}

/// A sink that records **when** each write happened, not merely what it contained.
///
/// The arrival time has to be taken here rather than after parsing, or the number reported would
/// include this test's own work -- which is harness delay, and is exactly what SC-001 excludes.
/// One write: when it reached the sink, and what it carried.
type Arrival = (u128, Vec<u8>);

#[derive(Clone, Default)]
struct StampingSink(Arc<Mutex<Vec<Arrival>>>);

impl Write for StampingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let at = monotonic_nanos();
        self.0.lock().expect("sink").push((at, buf.to_vec()));
        Ok(buf.len())
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

/// Every `STAMP <nanos>` the fixture wrote, paired with the time its frame reached the sink.
fn samples() -> Vec<f64> {
    let sink = StampingSink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner.clone());

    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_stamped")];
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
    service.adopt(TaskId("lat".into()), task.pid, task.control, task.output);

    // 120 writes 25 ms apart is about three seconds. The wait is on the fixture finishing.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let done = sink
            .0
            .lock()
            .expect("sink")
            .iter()
            .any(|(_, b)| String::from_utf8_lossy(b).contains("execution/onExit"));
        if done {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    service.close();

    let recorded = sink.0.lock().expect("sink").clone();
    let mut out = Vec::new();
    for (arrived, raw) in &recorded {
        let text = String::from_utf8_lossy(raw);
        // One frame can carry several stamped lines if the chunker coalesced them. Each line is
        // still its own sample, measured from its own stamp -- which is why the fixture stamps
        // per write rather than per frame.
        let Some(at) = text.find("\"data\":\"") else {
            continue;
        };
        let rest = &text[at + 8..];
        let Some(end) = rest.find('"') else { continue };
        let Ok(bytes) = apex_protocol::base64::decode(&rest[..end]) else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            if let Some(stamp) = line.strip_prefix("STAMP ") {
                if let Ok(sent) = stamp.trim().parse::<u128>() {
                    out.push((arrived.saturating_sub(sent)) as f64 / 1_000_000.0);
                }
            }
        }
    }
    out
}

/// The p99 by nearest-rank, which is the definition that needs no interpolation and cannot
/// report a value no sample actually had.
fn p99(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let rank = ((values.len() as f64) * 0.99).ceil() as usize;
    values[rank.saturating_sub(1).min(values.len() - 1)]
}

#[test]
fn output_reaches_the_boundary_within_the_interaction_budget() {
    let values = samples();

    assert!(
        values.len() >= MIN_SAMPLES,
        "SC-001 needs at least {MIN_SAMPLES} samples, measured {}",
        values.len()
    );

    let worst = values.iter().cloned().fold(f64::MIN, f64::max);
    let best = values.iter().cloned().fold(f64::MAX, f64::min);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let measured = p99(values.clone());

    println!("SC-001 samples: {}", values.len());
    println!("SC-001 min:  {best:.3} ms");
    println!("SC-001 mean: {mean:.3} ms");
    println!("SC-001 p99:  {measured:.3} ms (budget {BUDGET_MS} ms)");
    println!("SC-001 max:  {worst:.3} ms");
    println!("SC-001 headroom at p99: {:.3} ms", BUDGET_MS - measured);

    assert!(
        measured <= BUDGET_MS,
        "SC-001 p99 was {measured:.3} ms, over the {BUDGET_MS} ms budget"
    );
}
