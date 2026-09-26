//! Reports its terminal's size, and again whenever the size changes.
//!
//! FR-016 and SC-009 are about a resize reaching the process. The only thing that can answer
//! "did it arrive?" is the process itself asking the kernel, which is what this does: a
//! `TIOCGWINSZ` on its own descriptor, printed once at startup and once per `SIGWINCH`.
//!
//! Startup matters as much as the change. A `pty: true` task started without `cols`/`rows`
//! reads whatever the pseudo-terminal was created with, and §4.8 defaults that to 80 x 24
//! precisely so it is never the kernel's 0 x 0. The first line here is what proves it.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static RESIZED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_winch(_sig: libc::c_int) {
    RESIZED.store(true, Ordering::Relaxed);
}

fn size() -> (u16, u16) {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 {
        (ws.ws_col, ws.ws_row)
    } else {
        // Reported rather than hidden: a task with pipes instead of a terminal has no size,
        // and a test asserting on 0 x 0 should be able to tell that from a real 0 x 0.
        (0, 0)
    }
}

fn main() {
    unsafe {
        libc::signal(
            libc::SIGWINCH,
            on_winch as extern "C" fn(libc::c_int) as libc::sighandler_t,
        )
    };

    let mut out = std::io::stdout();
    let (cols, rows) = size();
    writeln!(out, "SIZE cols={cols} rows={rows}").expect("write");
    out.flush().expect("flush");

    loop {
        if RESIZED.swap(false, Ordering::Relaxed) {
            let (cols, rows) = size();
            writeln!(out, "RESIZED cols={cols} rows={rows}").expect("write");
            out.flush().expect("flush");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
