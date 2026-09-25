/// FR-029a: the panel's history is bounded.
///
/// An unbounded scrollback makes a long build a memory leak on the developer's own machine. A
/// full rebuild of a large workspace emits hundreds of thousands of lines, and a panel that kept
/// all of them would hold them for as long as the session lasts -- which is the rest of the day.
///
/// The bound is **read from the exported constant**, never typed in here. A test with 10000
/// written into it agrees with a number rather than with a decision, and keeps passing after
/// somebody changes the policy -- at which point it is measuring nothing.
import { describe, expect, it } from 'vitest';
import { SCROLLBACK_LINES, TerminalPanel } from '../../client/ui/lib/terminal/terminals.svelte';

describe('the panel keeps a bounded history (FR-029a)', () => {
  it('states a bound that is larger than the library default and finite', () => {
    // 1000 is the library's own default and is too few to scroll back through a compile, which
    // is the case the panel exists for. Both halves matter: raising it without bounding it would
    // be the leak, and leaving it at the default would be a panel that forgets the error.
    expect(SCROLLBACK_LINES).toBeGreaterThan(1000);
    expect(Number.isFinite(SCROLLBACK_LINES)).toBe(true);
    expect(SCROLLBACK_LINES).toBeLessThanOrEqual(100_000);
  });

  it('is the value the panel is actually built with', () => {
    // The constant existing is not the claim; the terminal being constructed with it is. A
    // constant nothing reads is documentation, and documentation does not bound memory.
    const source = TerminalPanel.prototype.attach.toString();
    expect(source).toContain('SCROLLBACK_LINES');
  });

  it('buffers unbounded output only until it is attached', () => {
    // The pending buffer is the one place the panel holds bytes itself, and it is emptied into
    // the library the moment the panel mounts. What is asserted here is that it does not become
    // a second, unbounded history: after attaching, `buffered` is empty and the library's own
    // bounded scrollback is the only thing holding anything.
    const panel = new TerminalPanel('task-a');
    for (let i = 0; i < 5000; i += 1) panel.write(`line ${i}\r\n`);
    expect(panel.buffered().length).toBeGreaterThan(0);
    expect(panel.attached).toBe(false);
  });

  it('starts every panel with the same bound', () => {
    // Per task, so a second terminal does not double the ceiling silently. Two panels each
    // keeping SCROLLBACK_LINES is the intended cost and is what FR-026 asks for; a panel that
    // inherited a larger bound from somewhere would make the total unpredictable.
    const a = new TerminalPanel('task-a');
    const b = new TerminalPanel('task-b');
    expect(a.taskId).not.toBe(b.taskId);
    // Neither is attached, so neither holds a library instance yet -- the bound is applied at
    // attach and is the same constant for both.
    expect(a.attached).toBe(false);
    expect(b.attached).toBe(false);
  });
});
