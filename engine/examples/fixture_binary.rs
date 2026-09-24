//! Writes a byte sequence that is not valid UTF-8.
//!
//! Without it SC-003 is vacuous: a pipeline that decodes to text and re-encodes passes every
//! ASCII fixture and loses exactly these bytes, substituting U+FFFD. quickstart §10 mutation 6
//! breaks the encoding deliberately, and this is what makes that mutation visible.

use std::io::Write;

/// A lone continuation byte, an unpaired surrogate's encoding, an overlong form, and a bare
/// 0xFF -- four different ways to be invalid, so a decoder that repairs one still fails here.
const NOT_UTF8: &[u8] = &[
    0x80, 0xED, 0xA0, 0x80, 0xC0, 0xAF, 0xFF, 0xFE, 0x00, 0x01, 0x7F,
];

fn main() {
    let mut out = std::io::stdout().lock();
    out.write_all(b"BINARY-BEGIN\n").expect("write");
    out.write_all(NOT_UTF8).expect("write");
    out.write_all(b"\nBINARY-END\n").expect("write");
    out.flush().expect("flush");
}
