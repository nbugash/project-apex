//! Writes 50 MiB as fast as the pipe will take it.
//!
//! SC-006's producer. It does not pace itself and does not flush per line: the point is to be
//! the fastest producer the control channel will ever carry, so that the interaction budget is
//! measured under the load it was written for rather than under a polite approximation of it.

use std::io::Write;

const TOTAL_BYTES: usize = 50 * 1024 * 1024;
const BLOCK: usize = 64 * 1024;

fn main() {
    let mut out = std::io::stdout().lock();
    let block = vec![b'x'; BLOCK];
    let mut written = 0usize;
    while written < TOTAL_BYTES {
        let take = BLOCK.min(TOTAL_BYTES - written);
        out.write_all(&block[..take]).expect("write");
        written += take;
    }
    out.flush().expect("flush");
}
