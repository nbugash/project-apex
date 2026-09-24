//! Crashes with a segmentation fault rather than an exit code.
//!
//! §9's core-dump check needs a real crash to find no dump for. A process that called
//! `process::exit(1)` would leave no dump either, and the check would pass having proven
//! nothing about `RLIMIT_CORE`.

use std::io::Write;

fn main() {
    let mut out = std::io::stdout();
    writeln!(out, "ABOUT-TO-CRASH pid={}", std::process::id()).expect("write");
    out.flush().expect("flush");

    // A null write, which the kernel answers with SIGSEGV. `write_volatile` so the optimiser
    // cannot reason the store away as unreachable.
    unsafe { std::ptr::null_mut::<u8>().write_volatile(1) };
}
