//! Reports the signal it caught and **keeps running**.
//!
//! US2.2 asserts an interrupted task is still alive five seconds later, which only means
//! anything if the fixture could have died. A process that exits on `SIGINT` would satisfy that
//! assertion by accident on any implementation, including one that sends nothing at all.

use std::io::Write;
use std::sync::atomic::{AtomicI32, Ordering};

static CAUGHT: AtomicI32 = AtomicI32::new(0);

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

    let mut out = std::io::stdout();
    writeln!(out, "READY pid={}", std::process::id()).expect("write");
    out.flush().expect("flush");

    loop {
        let sig = CAUGHT.swap(0, Ordering::Relaxed);
        if sig != 0 {
            writeln!(out, "CAUGHT signal={sig}").expect("write");
            out.flush().expect("flush");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
