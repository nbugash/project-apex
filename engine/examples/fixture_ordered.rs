//! Writes a monotonically numbered sequence and exits **immediately** after its last write.
//!
//! The numbering is what makes an ordering failure visible: a gap is a missing number and a
//! transposition is a number out of place, and neither is detectable in output that merely says
//! "some bytes arrived".
//!
//! Exiting immediately is the other half, and it is the point. A fixture that slept after its
//! last write would hand the delivery path all the slack it needs to get the ordering right by
//! luck, and SC-011 would pass for an implementation that reorders. quickstart §10's fourth
//! mutation is exactly this, so there is deliberately no flush-and-wait here: the last line and
//! the exit are as close together as the program can make them.

use std::io::Write;

const LINES: usize = 200;

fn main() {
    let mut out = std::io::stdout().lock();
    for n in 0..LINES {
        writeln!(out, "LINE {n}").expect("write");
    }
    // One flush, then straight out. Nothing waits.
    out.flush().expect("flush");
}
