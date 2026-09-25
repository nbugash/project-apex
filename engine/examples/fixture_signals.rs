//! Reports the signal it caught and **keeps running**.
//!
//! US2.2 asserts an interrupted task is still alive five seconds later, which only means
//! anything if the fixture could have died. A process that exits on `SIGINT` would satisfy that
//! assertion by accident on any implementation, including one that sends nothing at all.

//! It also **records what it read from stdin**, which is the other half of US2.2. With a
//! terminal, a `0x03` the client writes is turned into `SIGINT` by the line discipline and the
//! program never sees the byte; with pipes there is no line discipline and the byte arrives as
//! data. A fixture that only reported signals could not tell those apart, and the difference is
//! exactly what a panel has to branch on when it decides how to send an interrupt.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

static CAUGHT: AtomicI32 = AtomicI32::new(0);
/// How many `0x03` bytes have arrived as **data**. With a terminal this must stay zero.
static ETX_BYTES: AtomicUsize = AtomicUsize::new(0);
/// Every byte read, so "nothing arrived" is distinguishable from "nothing was read".
static BYTES_READ: AtomicUsize = AtomicUsize::new(0);

extern "C" fn handler(sig: libc::c_int) {
    // The only thing done in the handler is a relaxed store. Everything else -- allocating,
    // formatting, writing -- is not async-signal-safe, so the reporting happens in the loop.
    CAUGHT.store(sig, Ordering::Relaxed);
}

fn main() {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe {
            libc::signal(
                sig,
                handler as extern "C" fn(libc::c_int) as libc::sighandler_t,
            )
        };
    }

    // Read on its own thread so the reporting loop below keeps running while stdin blocks.
    std::thread::spawn(|| {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 256];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 {
                break;
            }
            BYTES_READ.fetch_add(n, Ordering::Relaxed);
            let etx = buf[..n].iter().filter(|b| **b == 0x03).count();
            if etx > 0 {
                ETX_BYTES.fetch_add(etx, Ordering::Relaxed);
            }
        }
    });

    let mut out = std::io::stdout();
    writeln!(out, "READY pid={}", std::process::id()).expect("write");
    out.flush().expect("flush");

    let mut reported = (0usize, 0usize);
    loop {
        let sig = CAUGHT.swap(0, Ordering::Relaxed);
        if sig != 0 {
            writeln!(out, "CAUGHT signal={sig}").expect("write");
            out.flush().expect("flush");
        }
        // Reported on change rather than every tick, so a test reading the transcript sees one
        // line per thing that happened instead of a stream it has to deduplicate.
        let now = (
            BYTES_READ.load(Ordering::Relaxed),
            ETX_BYTES.load(Ordering::Relaxed),
        );
        if now != reported {
            reported = now;
            writeln!(out, "INPUT bytes={} etx={}", now.0, now.1).expect("write");
            out.flush().expect("flush");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
