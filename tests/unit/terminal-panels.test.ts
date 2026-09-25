/// FR-026: two tasks are two terminals, and they share nothing.
///
/// The negative assertions are the half that matters. A single shared instance keyed by nothing
/// passes every positive claim -- output arrives, a resize takes effect, a release works -- and is
/// exactly the bug FR-026 exists to prevent. So each case asserts what the *other* panel did not
/// see, and the expected count is zero.
import { describe, expect, it } from 'vitest';
import {
  Terminals,
  TerminalPanel,
  DEFAULT_COLS,
  DEFAULT_ROWS,
} from '../../client/ui/lib/terminal/terminals.svelte';

/// `buffered` returns bytes, because output is bytes. These cases write ASCII, so decoding for
/// the comparison is safe here and is the test's own decision rather than the panel's.
const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

describe('terminal panels are one per task (FR-026)', () => {
  it('yields two instances for two task ids, and the same instance twice for one', () => {
    const terminals = new Terminals();
    const a = terminals.panel('task-a');
    const b = terminals.panel('task-b');
    expect(a).not.toBe(b);
    expect(a.taskId).toBe('task-a');
    expect(b.taskId).toBe('task-b');
    // Identity, not merely equality: asking twice must reach the panel that already holds the
    // scrollback, or every re-render would start an empty terminal.
    expect(terminals.panel('task-a')).toBe(a);
    expect(terminals.panels.length).toBe(2);
  });

  it('delivers output to one panel and zero of it to the other', () => {
    const terminals = new Terminals();
    const a = terminals.panel('task-a');
    const b = terminals.panel('task-b');
    a.write('compiling apex-engine\r\n');
    a.write('Finished in 11.07s\r\n');
    expect(text(a.buffered())).toBe('compiling apex-engine\r\nFinished in 11.07s\r\n');
    expect(b.buffered().length).toBe(0);
    expect(text(b.buffered())).not.toContain('apex-engine');
  });

  it('resizes one panel and leaves the other at its own dimensions', () => {
    const terminals = new Terminals();
    const a = terminals.panel('task-a');
    const b = terminals.panel('task-b');
    a.resize(200, 60);
    expect(a.cols).toBe(200);
    expect(a.rows).toBe(60);
    expect(b.cols).toBe(DEFAULT_COLS);
    expect(b.rows).toBe(DEFAULT_ROWS);
  });

  it('ignores a zero dimension rather than forwarding it', () => {
    // Some programs read a zero dimension as "no terminal" and change what they print. A resize
    // that arrives as zero is a measurement of an unmounted element, not an instruction.
    const panel = new TerminalPanel('task-a');
    panel.resize(120, 40);
    panel.resize(0, 40);
    panel.resize(120, 0);
    expect(panel.cols).toBe(120);
    expect(panel.rows).toBe(40);
  });

  it('releases one panel and leaves the other whole', () => {
    const terminals = new Terminals();
    const a = terminals.panel('task-a');
    const b = terminals.panel('task-b');
    a.write('gone with it\r\n');
    b.write('still here\r\n');
    terminals.release('task-a');
    expect(terminals.has('task-a')).toBe(false);
    expect(terminals.has('task-b')).toBe(true);
    expect(terminals.panels.length).toBe(1);
    expect(text(b.buffered())).toBe('still here\r\n');
    // Asking again after a release is a new panel, not the released one resurrected.
    expect(terminals.panel('task-a')).not.toBe(a);
    expect(terminals.panel('task-a').buffered().length).toBe(0);
  });

  it('releasing an unknown id changes nothing', () => {
    const terminals = new Terminals();
    terminals.panel('task-a').write('untouched\r\n');
    terminals.release('never-existed');
    expect(terminals.panels.length).toBe(1);
    expect(text(terminals.panel('task-a').buffered())).toBe('untouched\r\n');
  });
});
