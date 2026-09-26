//! Writes a 4 MiB line containing no newline at all.
//!
//! SC-005's case. A chunker that flushes on newlines looks correct against every ordinary
//! program and stalls forever here, so the absence of the newline is the whole fixture.

use std::io::Write;

const LINE_BYTES: usize = 4 * 1024 * 1024;

fn main() {
    let mut out = std::io::stdout().lock();
    // A repeating pattern rather than one byte, so a test can tell truncation from duplication
    // by looking at where the sequence breaks.
    let pattern: Vec<u8> = (b'a'..=b'z').collect();
    let mut written = 0usize;
    while written < LINE_BYTES {
        let take = pattern.len().min(LINE_BYTES - written);
        out.write_all(&pattern[..take]).expect("write");
        written += take;
    }
    // The newline comes after the 4 MiB, so the line itself is 4 MiB long.
    out.write_all(b"\n").expect("write");
    out.flush().expect("flush");
}
