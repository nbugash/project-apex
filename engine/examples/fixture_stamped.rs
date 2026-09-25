//! Writes its own send time, so latency can be measured from the producer rather than the harness.
//!
//! SC-001 is "from the task writing a byte to that byte reaching the client". Neither end of that
//! can be observed by a test that only knows when it started the process: everything between the
//! spawn and the first write belongs to exec and to the loader, and counting it would report the
//! cost of starting a program as the cost of delivering its output.
//!
//! `CLOCK_MONOTONIC` is system-wide on Linux, so a stamp taken here is directly comparable with
//! one taken in the test process. A wall clock is not -- it can step -- and a per-process clock
//! would not be comparable at all.

use std::io::Write;

/// Enough samples that a p99 is a measurement rather than a coincidence. SC-001 asks for at
/// least 100; the extras absorb the first few, which carry page-fault cost that is real but is
/// not what the criterion is about.
const WRITES: usize = 120;

/// Longer than the chunker's time bound, so each write leaves on its own timer rather than
/// being coalesced with its neighbours into one frame whose stamp belongs to the oldest of them.
const GAP: std::time::Duration = std::time::Duration::from_millis(25);

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

fn main() {
    let mut out = std::io::stdout();
    for _ in 0..WRITES {
        // Stamp and write in the same breath, and flush, so the recorded time is the time the
        // bytes left rather than the time they were formatted.
        writeln!(out, "STAMP {}", monotonic_nanos()).expect("write");
        out.flush().expect("flush");
        std::thread::sleep(GAP);
    }
}
