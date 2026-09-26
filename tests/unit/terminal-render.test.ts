/// A chunk boundary is where the engine ended a frame. It means nothing.
///
/// The engine splits output on a byte count and a timer (§4.8), neither of which knows anything
/// about escape sequences or character boundaries. So a boundary lands mid-sequence and
/// mid-character routinely, and a panel that treats one as a delimiter renders a stray `[0m` or a
/// replacement character that the engine never sent.
///
/// These cases assert on **bytes**, not on rendering. Whether the terminal library draws green
/// where the sequence said green is the library's claim and SC-002's end-to-end spec; what this
/// application must not do is hand it different bytes from the ones that arrived. A `TextDecoder`
/// without `{ stream: true }` anywhere on that path is the whole failure mode, and it is invisible
/// at the engine because the bytes left it intact.
import { describe, expect, it } from 'vitest';
import { applyChunk, TerminalPanel } from '../../client/ui/lib/terminal/terminals.svelte';
import { decodeBase64, encodeBase64 } from '../../client/ui/lib/terminal/wire';

/// Base64 exactly as §4.8 carries it, so the test drives the real entry point.
const wire = (bytes: number[]) => encodeBase64(new Uint8Array(bytes));

/// Split `bytes` at `at` and deliver the two halves as two frames.
const deliverSplit = (bytes: number[], at: number): Uint8Array => {
  const panel = new TerminalPanel('task-a');
  applyChunk(panel, wire(bytes.slice(0, at)));
  applyChunk(panel, wire(bytes.slice(at)));
  return panel.buffered();
};

describe('chunk boundaries are not delimiters (FR-022, SC-003)', () => {
  it('survives a boundary falling mid-escape-sequence', () => {
    // SGR green, "ok", SGR reset. Split inside the opening sequence, between `[` and `3`.
    const bytes = [0x1b, 0x5b, 0x33, 0x32, 0x6d, 0x6f, 0x6b, 0x1b, 0x5b, 0x30, 0x6d];
    expect([...deliverSplit(bytes, 2)]).toEqual(bytes);
    // Every interior split, not just one: the boundary is wherever the timer happened to fire.
    for (let at = 1; at < bytes.length; at += 1) {
      expect([...deliverSplit(bytes, at)]).toEqual(bytes);
    }
  });

  it('survives a boundary falling mid-UTF-8-character', () => {
    // U+276F HEAVY RIGHT-POINTING ANGLE QUOTATION MARK ORNAMENT -- the prompt glyph -- is three
    // bytes. Splitting it is the case a non-streaming decode turns into U+FFFD.
    const bytes = [0x24, 0xe2, 0x9d, 0xaf, 0x20];
    for (let at = 1; at < bytes.length; at += 1) {
      const out = deliverSplit(bytes, at);
      expect([...out]).toEqual(bytes);
      // The substitution this is really about. U+FFFD encodes as EF BF BD; zero of them.
      expect([...out].join(',')).not.toContain('239,191,189');
    }
  });

  it('carries bytes that are not valid UTF-8 at all', () => {
    // A lone continuation byte and a bare 0xFF are not a character in any encoding. A compiler
    // emitting a filename in some other encoding produces exactly this, and it must arrive whole.
    const bytes = [0xff, 0xfe, 0x80, 0x41, 0x00, 0x42];
    const panel = new TerminalPanel('task-a');
    applyChunk(panel, wire(bytes));
    expect([...panel.buffered()]).toEqual(bytes);
  });

  it('round-trips every byte value through the wire form', () => {
    // 0..255 with no gaps. An encoder that goes via a text encoding mangles the high half, and a
    // test using only printable ASCII would never notice.
    const all = Array.from({ length: 256 }, (_, i) => i);
    expect([...decodeBase64(encodeBase64(new Uint8Array(all)))]).toEqual(all);
  });

  it('keeps frames in arrival order', () => {
    const panel = new TerminalPanel('task-a');
    applyChunk(panel, wire([0x31]));
    applyChunk(panel, wire([0x32]));
    applyChunk(panel, wire([0x33]));
    expect([...panel.buffered()]).toEqual([0x31, 0x32, 0x33]);
  });
});
