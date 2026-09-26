//! Base64 for the wire, without a dependency for a table lookup.
//!
//! Lives in `protocol` because it is a property of the frame rather than of either side. F003
//! wrote the encoder privately inside the engine's workspace use case, which was right while
//! `workspace/readFile` was the only thing that needed it. F010 needs the **decoder** as well --
//! `execution/writeStdin` arrives encoded -- and a wire format with two implementations is a
//! wire format that will eventually disagree with itself.

const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if c.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Why a string was not base64.
///
/// An error and not a lossy decode. The whole reason output is encoded is that a task's bytes
/// are not text (FR-009); a decoder that substituted a replacement byte for a character it did
/// not recognise would reintroduce exactly the corruption the encoding exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// A character outside the alphabet, or `=` somewhere other than the end.
    BadCharacter,
    /// Base64 is four characters to three bytes; a length that is not a multiple of four cannot
    /// be a whole encoding.
    BadLength,
}

pub fn decode(text: &str) -> Result<Vec<u8>, DecodeError> {
    let s = text.as_bytes();
    // `% 4` and not `is_multiple_of`, which is stable only since 1.87 against a declared
    // MSRV of 1.75. clippy's incompatible-msrv lint is what caught it.
    if s.len() % 4 != 0 {
        return Err(DecodeError::BadLength);
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.chunks(4) {
        let mut n: u32 = 0;
        let mut pad = 0usize;
        for (i, &ch) in chunk.iter().enumerate() {
            // Padding is only ever the **trailing** one or two characters of the last chunk.
            // `Zm=v` has a `=` in a legal position followed by data, and an earlier version of
            // this accepted it and returned two plausible bytes -- which is the worst outcome
            // available, because a caller cannot tell a wrong answer from a right one.
            if pad > 0 && ch != b'=' {
                return Err(DecodeError::BadCharacter);
            }
            let six = match ch {
                b'A'..=b'Z' => ch - b'A',
                b'a'..=b'z' => ch - b'a' + 26,
                b'0'..=b'9' => ch - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' if i >= 2 => {
                    pad += 1;
                    0
                }
                _ => return Err(DecodeError::BadCharacter),
            };
            n = (n << 6) | six as u32;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648's own vectors. Every padding case appears, which is where a hand-written codec
    /// goes wrong.
    #[test]
    fn the_known_vectors_round_trip() {
        for (raw, encoded) in [
            (&b""[..], ""),
            (&b"f"[..], "Zg=="),
            (&b"fo"[..], "Zm8="),
            (&b"foo"[..], "Zm9v"),
            (&b"foob"[..], "Zm9vYg=="),
            (&b"fooba"[..], "Zm9vYmE="),
            (&b"foobar"[..], "Zm9vYmFy"),
        ] {
            assert_eq!(encode(raw), encoded, "encoding {raw:?}");
            assert_eq!(decode(encoded).expect("decode"), raw, "decoding {encoded}");
        }

        // Carried from F003's test when the encoder moved here. Bytes with the high bit set are
        // the case a text-assuming encoder mangles, and all-zero is the case an off-by-one
        // table index still encodes to something plausible.
        assert_eq!(encode(&[0xff, 0xfe, 0xfd]), "//79");
        assert_eq!(encode(&[0x00, 0x00, 0x00]), "AAAA");
    }

    /// The vectors above are not redundant with the round trip below them.
    ///
    /// A round trip passes whenever `decode` is the inverse of `encode`, including when both
    /// use the same wrong alphabet. Only a known vector pins the encoding to the standard one.
    #[test]
    fn a_round_trip_alone_would_not_pin_the_alphabet() {
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn bytes_that_are_not_text_survive_the_round_trip() {
        // The case the encoding exists for: no part of this is valid UTF-8.
        let raw: Vec<u8> = vec![0x00, 0x80, 0xFF, 0xFE, 0xED, 0xA0, 0x80];
        assert_eq!(decode(&encode(&raw)).expect("decode"), raw);
    }

    #[test]
    fn every_byte_value_survives() {
        let raw: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&raw)).expect("decode"), raw);
    }

    #[test]
    fn a_malformed_string_is_an_error_rather_than_a_guess() {
        assert_eq!(decode("Zm9v!"), Err(DecodeError::BadLength));
        assert_eq!(decode("Zm9"), Err(DecodeError::BadLength));
        assert_eq!(decode("Zm=v"), Err(DecodeError::BadCharacter));
        assert_eq!(decode("Zm9 "), Err(DecodeError::BadCharacter));
    }
}
