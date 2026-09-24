//! §4.6's ordering, in the direction F010 floods (T034).
//!
//! Every assertion here is on the order frames reach the **sink**, never on the order calls
//! returned. Call order is what the test controls; sink order is what §4.6 requires. A test
//! asserting the former passes with the gate deleted.

use apex_engine::adapters::outbound::frame_writer::{FrameWriter, CONSECUTIVE_YIELDS};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A sink that records what reaches it, and can be held shut.
///
/// Shut means a writer **blocks inside** the sink, holding the write mutex. That is how the
/// test parks an interactive writer in a way that raises the waiting count, which is the
/// condition a bulk writer is supposed to yield to.
#[derive(Default)]
struct Recorder {
    frames: Mutex<Vec<String>>,
    open: Mutex<bool>,
    opened: Condvar,
}

impl Recorder {
    fn new(open: bool) -> Arc<Self> {
        Arc::new(Self {
            frames: Mutex::new(Vec::new()),
            open: Mutex::new(open),
            opened: Condvar::new(),
        })
    }

    fn release(&self) {
        *self.open.lock().expect("open") = true;
        self.opened.notify_all();
    }

    fn frames(&self) -> Vec<String> {
        self.frames.lock().expect("frames").clone()
    }
}

/// The handle a `FrameWriter` owns. Shares one `Recorder` with the test.
struct Handle(Arc<Recorder>);

impl Write for Handle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut open = self.0.open.lock().expect("open");
        while !*open {
            open = self.0.opened.wait(open).expect("wait");
        }
        drop(open);
        self.0
            .frames
            .lock()
            .expect("frames")
            .push(String::from_utf8_lossy(buf).into_owned());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn writer_on(rec: &Arc<Recorder>) -> Arc<FrameWriter> {
    Arc::new(FrameWriter::new(Box::new(Handle(Arc::clone(rec)))))
}

/// Wait until `cond` holds, or fail. Never a bare sleep: a sleep long enough to be reliable is
/// long enough to make the suite slow, and one short enough to be quick is flaky.
fn until(cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("condition never held");
}

#[test]
fn a_bulk_writer_parked_at_the_gate_is_overtaken_by_a_later_interactive_one() {
    // Repeated, because a plain Mutex has no fairness guarantee: with the gate deleted the late
    // interactive frame lands second on *some* schedules, and a single run that happened to
    // order things correctly is not evidence of anything.
    for attempt in 0..25 {
        let rec = Recorder::new(false); // shut: the first writer will block inside the sink
        let w = writer_on(&rec);

        let started = Arc::new(AtomicBool::new(false));
        let i0 = {
            let (w, started) = (Arc::clone(&w), Arc::clone(&started));
            std::thread::spawn(move || {
                started.store(true, Ordering::SeqCst);
                w.write(b"I0").expect("i0");
            })
        };
        // I0 is now inside the sink holding the write mutex, with the waiting count raised.
        until(|| started.load(Ordering::SeqCst));
        std::thread::sleep(Duration::from_millis(5));

        // B yields at the gate -- not inside the sink, which is the distinction that makes this
        // test mean anything. A writer parked inside the sink holds the mutex, and nothing
        // could overtake it however fair the gate was.
        let b = {
            let w = Arc::clone(&w);
            std::thread::spawn(move || w.write_bulk(b"B").expect("b"))
        };
        std::thread::sleep(Duration::from_millis(5));

        // I1 arrives *after* B called, and must still reach the sink before it.
        let i1 = {
            let w = Arc::clone(&w);
            std::thread::spawn(move || w.write(b"I1").expect("i1"))
        };
        std::thread::sleep(Duration::from_millis(5));

        rec.release();
        for h in [i0, i1, b] {
            h.join().expect("join");
        }

        let frames = rec.frames();
        assert_eq!(frames.len(), 3, "attempt {attempt}: {frames:?}");
        let bulk_at = frames
            .iter()
            .position(|f| f == "B")
            .expect("B reached the sink");
        assert_eq!(
            bulk_at, 2,
            "attempt {attempt}: the bulk frame must be last although it was requested second: {frames:?}"
        );
    }
}

#[test]
fn the_anti_starvation_bound_releases_a_yielding_writer() {
    let rec = Recorder::new(true);
    let w = writer_on(&rec);

    // An interactive writer kept waiting continuously: each one raises the count again before
    // the bulk writer can act on the count reaching zero.
    let stop = Arc::new(AtomicBool::new(false));
    let pressure = {
        let (w, stop) = (Arc::clone(&w), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut n = 0u32;
            while !stop.load(Ordering::SeqCst) {
                w.write(format!("I{n}").as_bytes()).expect("interactive");
                n += 1;
                std::thread::sleep(Duration::from_micros(200));
            }
        })
    };

    let bulk = {
        let w = Arc::clone(&w);
        std::thread::spawn(move || w.write_bulk(b"B").expect("bulk"))
    };

    // It must get through despite the pressure. Without the bound this join never returns.
    bulk.join().expect("the bulk writer never got through");
    stop.store(true, Ordering::SeqCst);
    pressure.join().expect("join");

    let frames = rec.frames();
    let bulk_at = frames
        .iter()
        .position(|f| f == "B")
        .expect("B reached the sink");
    // Read from the constant the source exports, never typed as 8: plan.md's Fixed Quantities
    // owns the number, and a test carrying its own copy passes after somebody changes it.
    assert!(
        bulk_at <= CONSECUTIVE_YIELDS as usize + 2,
        "the bound should release it after about {CONSECUTIVE_YIELDS} interactive frames, \
         but it landed at {bulk_at}: {frames:?}"
    );
}

#[test]
fn two_producers_never_interleave_one_frame() {
    // What the mutex is still there for. §4.6: a frame interleaved with a reply is a corrupt
    // stream rather than a slow one.
    let rec = Recorder::new(true);
    let w = writer_on(&rec);

    let a = {
        let w = Arc::clone(&w);
        std::thread::spawn(move || {
            for _ in 0..200 {
                w.write(b"AAAAAAAAAAAAAAAA").expect("a");
            }
        })
    };
    let b = {
        let w = Arc::clone(&w);
        std::thread::spawn(move || {
            for _ in 0..200 {
                w.write_bulk(b"BBBBBBBBBBBBBBBB").expect("b");
            }
        })
    };
    a.join().expect("join");
    b.join().expect("join");

    let frames = rec.frames();
    assert_eq!(frames.len(), 400);
    for f in &frames {
        assert!(
            f == "AAAAAAAAAAAAAAAA" || f == "BBBBBBBBBBBBBBBB",
            "a frame was torn: {f:?}"
        );
    }
}
