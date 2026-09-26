//! Reports whether it is attached to a terminal, then writes to both streams.
//!
//! The stderr write is deliberately loud. SC-028 asserts that a task given a terminal delivers
//! **zero** bytes on the error stream, and a zero that could never have been anything else is
//! not a measurement. This fixture makes that zero able to fail.

use std::io::Write;

fn main() {
    // `isatty` is the question a process actually asks, and the answer is the whole point of
    // FR-002: with a pseudo-terminal it must be told yes.
    let stdout_is_tty = unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1;
    let stderr_is_tty = unsafe { libc::isatty(libc::STDERR_FILENO) } == 1;

    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    writeln!(out, "isatty(stdout)={stdout_is_tty}").expect("stdout");
    writeln!(out, "STDOUT-MARKER").expect("stdout");
    out.flush().expect("flush stdout");

    writeln!(err, "isatty(stderr)={stderr_is_tty}").expect("stderr");
    writeln!(err, "STDERR-MARKER").expect("stderr");
    err.flush().expect("flush stderr");
}
