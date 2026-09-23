//! `Content-Length` framing over the child process's stdio.
//!
//! The format is §4.1 of the system specification and is not restated here. What this module
//! owns is what an implementation must *do* about that format — see
//! specs/003-ssh-transport-core/contracts/framing.md.
//!
//! The child's stdout is untrusted input. The single most important property is that a
//! refused frame does not desynchronise the stream: a reader that misaligns turns one bad
//! frame into every subsequent frame being garbage, which presents as the engine having gone
//! insane rather than as one malformed message.

use bytes::{Buf, BytesMut};

/// §4.1's frame cap. It lives beside the codec because it *is* part of the format: a reader
/// that enforces a different cap from the writer is a reader that refuses valid frames.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

const HEADER: &str = "Content-Length: ";
const SEPARATOR: &[u8] = b"\r\n\r\n";

#[derive(Debug, PartialEq, Eq)]
pub struct Frame(pub String);

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A declared length beyond the cap. Refused **before** allocating.
    TooLarge(usize),
    /// A header that is not a header, or a body that is not JSON.
    Malformed(String),
}

#[derive(Default)]
pub struct FrameCodec;

impl FrameCodec {
    pub fn new() -> Self {
        Self
    }

    /// Encode one frame. Header and body are one unit: the length has already promised what
    /// follows, so a partially written frame corrupts the stream.
    pub fn encode(&self, body: &str) -> Result<Vec<u8>, FrameError> {
        if body.len() > MAX_FRAME_BYTES {
            return Err(FrameError::TooLarge(body.len()));
        }
        let mut out = Vec::with_capacity(body.len() + 32);
        out.extend_from_slice(HEADER.as_bytes());
        out.extend_from_slice(body.len().to_string().as_bytes());
        out.extend_from_slice(SEPARATOR);
        out.extend_from_slice(body.as_bytes());
        Ok(out)
    }

    /// Decode the next frame, if a whole one is present.
    ///
    /// `Ok(None)` means more bytes are needed. On `Ok(Some)` the buffer is positioned at the
    /// next frame boundary. On `Err` the buffer is *still* positioned at a boundary — that
    /// is what keeps one bad frame from poisoning everything after it.
    pub fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Frame>, FrameError> {
        let Some(sep) = find(buf, SEPARATOR) else {
            // No complete header yet. Guard against a peer that never sends the separator:
            // a header this long is not a header.
            if buf.len() > 1024 {
                let n = buf.len();
                buf.clear();
                return Err(FrameError::Malformed(format!(
                    "no header separator in {n} bytes"
                )));
            }
            return Ok(None);
        };

        let header = std::str::from_utf8(&buf[..sep])
            .map_err(|_| FrameError::Malformed("header is not utf-8".into()))?
            .to_string();

        let Some(digits) = header.trim_start().strip_prefix(HEADER) else {
            buf.advance(sep + SEPARATOR.len());
            return Err(FrameError::Malformed(format!("bad header: {header:?}")));
        };

        let len: usize = match digits.trim().parse() {
            Ok(n) => n,
            Err(_) => {
                buf.advance(sep + SEPARATOR.len());
                return Err(FrameError::Malformed(format!("bad length: {digits:?}")));
            }
        };

        // Before allocating, not after. Checking the cap after reserving the buffer would
        // defeat the defence entirely — the allocation is the harm.
        if len > MAX_FRAME_BYTES {
            buf.advance(sep + SEPARATOR.len());
            return Err(FrameError::TooLarge(len));
        }

        let start = sep + SEPARATOR.len();
        if buf.len() < start + len {
            return Ok(None); // body still arriving
        }

        let body = buf[start..start + len].to_vec();
        buf.advance(start + len);

        let text = String::from_utf8(body)
            .map_err(|_| FrameError::Malformed("body is not utf-8".into()))?;
        if serde_json::from_str::<serde_json::Value>(&text).is_err() {
            return Err(FrameError::Malformed("body is not json".into()));
        }
        Ok(Some(Frame(text)))
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> Vec<u8> {
        FrameCodec::new().encode(body).unwrap()
    }

    fn buf(bytes: &[u8]) -> BytesMut {
        BytesMut::from(bytes)
    }

    /// A normal frame after a hostile one is how alignment is proven. A test that only
    /// asserts the bad frame was rejected passes against a reader that silently
    /// desynchronised.
    fn assert_still_aligned(codec: &mut FrameCodec, b: &mut BytesMut) {
        b.extend_from_slice(&frame(r#"{"ok":true}"#));
        assert_eq!(
            codec.decode(b),
            Ok(Some(Frame(r#"{"ok":true}"#.into()))),
            "the stream desynchronised: a later, valid frame was not read"
        );
    }

    #[test]
    fn reads_a_whole_frame() {
        let mut c = FrameCodec::new();
        let mut b = buf(&frame(r#"{"a":1}"#));
        assert_eq!(c.decode(&mut b), Ok(Some(Frame(r#"{"a":1}"#.into()))));
        assert_eq!(c.decode(&mut b), Ok(None));
    }

    // --- the three rows that are not hostile input, and matter most ---

    #[test]
    fn reassembles_a_header_split_across_reads() {
        let mut c = FrameCodec::new();
        let whole = frame(r#"{"a":1}"#);
        let (head, tail) = whole.split_at(8);
        let mut b = buf(head);
        assert_eq!(
            c.decode(&mut b),
            Ok(None),
            "a partial header is not an error"
        );
        b.extend_from_slice(tail);
        assert_eq!(c.decode(&mut b), Ok(Some(Frame(r#"{"a":1}"#.into()))));
    }

    #[test]
    fn reassembles_a_body_split_across_reads() {
        let mut c = FrameCodec::new();
        let whole = frame(r#"{"value":"abcdefghij"}"#);
        let cut = whole.len() - 5;
        let mut b = buf(&whole[..cut]);
        assert_eq!(c.decode(&mut b), Ok(None), "a partial body is not an error");
        b.extend_from_slice(&whole[cut..]);
        assert_eq!(
            c.decode(&mut b),
            Ok(Some(Frame(r#"{"value":"abcdefghij"}"#.into())))
        );
    }

    #[test]
    fn delivers_two_frames_arriving_in_one_read() {
        let mut c = FrameCodec::new();
        let mut b = buf(&frame(r#"{"n":1}"#));
        b.extend_from_slice(&frame(r#"{"n":2}"#));
        assert_eq!(c.decode(&mut b), Ok(Some(Frame(r#"{"n":1}"#.into()))));
        assert_eq!(c.decode(&mut b), Ok(Some(Frame(r#"{"n":2}"#.into()))));
        assert_eq!(c.decode(&mut b), Ok(None));
    }

    // --- hostile input ---

    /// T015. The cap is checked before allocating; a declared gigabyte must not reserve one.
    #[test]
    fn refuses_a_declared_length_above_the_cap_without_allocating() {
        let mut c = FrameCodec::new();
        let huge = MAX_FRAME_BYTES + 1;
        let mut b = buf(format!("{HEADER}{huge}\r\n\r\n").as_bytes());
        assert_eq!(c.decode(&mut b), Err(FrameError::TooLarge(huge)));
        assert_still_aligned(&mut c, &mut b);
    }

    #[test]
    fn refuses_a_body_that_is_not_json_and_keeps_reading() {
        let mut c = FrameCodec::new();
        let body = "not json at all";
        let mut b = buf(format!("{HEADER}{}\r\n\r\n{body}", body.len()).as_bytes());
        assert!(matches!(c.decode(&mut b), Err(FrameError::Malformed(_))));
        assert_still_aligned(&mut c, &mut b);
    }

    #[test]
    fn refuses_a_header_that_is_not_a_header_and_keeps_reading() {
        let mut c = FrameCodec::new();
        let mut b = buf(b"Content-Bogus: 4\r\n\r\n");
        assert!(matches!(c.decode(&mut b), Err(FrameError::Malformed(_))));
        assert_still_aligned(&mut c, &mut b);
    }

    #[test]
    fn refuses_a_length_that_is_not_a_number_and_keeps_reading() {
        let mut c = FrameCodec::new();
        let mut b = buf(b"Content-Length: many\r\n\r\n");
        assert!(matches!(c.decode(&mut b), Err(FrameError::Malformed(_))));
        assert_still_aligned(&mut c, &mut b);
    }

    #[test]
    fn a_length_that_never_arrives_is_incomplete_not_malformed() {
        let mut c = FrameCodec::new();
        let mut b = buf(format!("{HEADER}64\r\n\r\nonly a few bytes").as_bytes());
        // Incomplete, not an error: at EOF the caller treats this as a lost connection.
        assert_eq!(c.decode(&mut b), Ok(None));
    }

    #[test]
    fn refuses_an_endless_header_rather_than_buffering_forever() {
        let mut c = FrameCodec::new();
        let mut b = buf(&vec![b'x'; 2048]);
        assert!(matches!(c.decode(&mut b), Err(FrameError::Malformed(_))));
    }

    #[test]
    fn encoding_refuses_an_oversized_payload_before_producing_bytes() {
        let c = FrameCodec::new();
        let big = "x".repeat(MAX_FRAME_BYTES + 1);
        assert_eq!(c.encode(&big), Err(FrameError::TooLarge(big.len())));
    }

    #[test]
    fn round_trips_at_the_cap_boundary() {
        let c = FrameCodec::new();
        let body = format!(r#"{{"v":"{}"}}"#, "x".repeat(1000));
        let mut b = buf(&c.encode(&body).unwrap());
        assert_eq!(FrameCodec::new().decode(&mut b), Ok(Some(Frame(body))));
    }
}
