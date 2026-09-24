//! Echoes stdin to stdout byte for byte, control bytes included.
//!
//! Deliberately not line-oriented and deliberately not text. FR-014 and FR-009 are about bytes
//! arriving unchanged, and a fixture that read lines or decoded UTF-8 would quietly repair the
//! very corruption the test exists to detect.

use std::io::{Read, Write};

fn main() {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut buf = [0u8; 4096];

    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                stdout.write_all(&buf[..n]).expect("write");
                // Flushed per read rather than per line: a shell prompt never ends in a newline,
                // and buffering until one would make an interactive echo look like a hang.
                stdout.flush().expect("flush");
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}
