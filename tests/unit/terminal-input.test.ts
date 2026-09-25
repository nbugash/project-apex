/// What a person types leaves the panel unchanged.
///
/// SC-007's client half. The engine's tests prove a task gets back exactly what was written to
/// it; these prove the panel wrote exactly what was typed. A substitution introduced here is
/// invisible from the engine, which faithfully delivers whatever it was handed.
import { describe, expect, it } from 'vitest';
import { encodeBase64, encodeInput, decodeBase64 } from '../../client/ui/lib/terminal/wire';

/// U+FFFD, encoded. Its presence anywhere on this path is the symptom.
const REPLACEMENT = [0xef, 0xbf, 0xbd];

const bytesOf = (base64: string) => [...decodeBase64(base64)];

describe('keystrokes leave the panel byte for byte (SC-007)', () => {
  it('carries control bytes without translating them', () => {
    // 0x03 interrupt, 0x04 end of transmission, 0x1b escape, 0x7f delete, and both line endings.
    // A path that translated \r to \n, or trimmed, changes the length first.
    const control = '\u0003\u0004\u001b\u007f\r\n';
    expect(bytesOf(encodeInput(control))).toEqual([0x03, 0x04, 0x1b, 0x7f, 0x0d, 0x0a]);
  });

  it('carries a NUL rather than truncating at it', () => {
    // A path built around C strings stops here, and every assertion about earlier bytes passes.
    expect(bytesOf(encodeInput('a\u0000b'))).toEqual([0x61, 0x00, 0x62]);
  });

  it('encodes every byte value without substitution', () => {
    // 0..255 as raw bytes, which is what a paste of arbitrary content amounts to. An encoder
    // that goes via a text encoding mangles the high half; a test using printable ASCII would
    // never notice, and most of what anyone types is printable ASCII.
    const all = new Uint8Array(Array.from({ length: 256 }, (_, i) => i));
    expect(bytesOf(encodeBase64(all))).toEqual([...all]);
  });

  it('introduces zero replacement characters', () => {
    // The failure this is really about: a `TextDecoder` anywhere on the way out turns anything
    // that is not valid UTF-8 into U+FFFD, and the engine cannot tell.
    const all = new Uint8Array(Array.from({ length: 256 }, (_, i) => i));
    const out = bytesOf(encodeBase64(all));
    let found = 0;
    for (let i = 0; i + 2 < out.length; i += 1) {
      if (
        out[i] === REPLACEMENT[0] &&
        out[i + 1] === REPLACEMENT[1] &&
        out[i + 2] === REPLACEMENT[2]
      ) {
        found += 1;
      }
    }
    expect(found).toBe(0);
  });

  it('encodes a multi-byte character as its UTF-8 bytes, not as a code point', () => {
    // The prompt glyph. A key event is a string, so this is the one place a conversion to bytes
    // has to happen, and it has to be UTF-8 rather than anything clever.
    expect(bytesOf(encodeInput('❯'))).toEqual([0xe2, 0x9d, 0xaf]);
  });

  it('encodes nothing as nothing', () => {
    // An empty write is a legal frame. It must not become a byte, and must not throw.
    expect(bytesOf(encodeInput(''))).toEqual([]);
  });

  it('handles a paste larger than the call stack would take', () => {
    // `String.fromCharCode(...bytes)` spreads the whole array onto the stack and throws on a
    // large paste. A person pasting a stack trace into a REPL hits exactly this.
    const big = new Uint8Array(200_000).fill(0x41);
    const out = bytesOf(encodeBase64(big));
    expect(out.length).toBe(200_000);
    expect(out[0]).toBe(0x41);
    expect(out[out.length - 1]).toBe(0x41);
  });
});
