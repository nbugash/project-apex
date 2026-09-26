/// What an ending means, which is a rule rather than a rendering.
///
/// Each case here is one that reads as obviously right until it is written down. The protocol
/// spent a field keeping a signalled death apart from an exit status, and every one of these
/// exists to stop that field being thrown away one layer up.
import { describe, expect, it } from 'vitest';
import { describeEnding } from '../../client/ui/lib/terminal/ending';

describe('describing how a task ended', () => {
  it('reads a clean exit as clean', () => {
    const d = describeEnding({ exitCode: 0, signal: null });
    expect(d.badge).toBe('0');
    expect(d.spoken).toBe('exited 0');
    expect(d.ok).toBe(true);
  });

  it('reads a non-zero exit as not clean, and says which', () => {
    // The code is the badge, so the tab says *how* it ended and not merely that it did (FR-029).
    const d = describeEnding({ exitCode: 1, signal: null });
    expect(d.badge).toBe('1');
    expect(d.ok).toBe(false);
  });

  it('names a signal rather than manufacturing 128 + n', () => {
    // A shell reports a signalled death as an exit status because a status is all it has. This
    // protocol carries the name, and rebuilding the shell's convention on top of it would throw
    // away the distinction the field exists for.
    const d = describeEnding({ exitCode: null, signal: 'SIGTERM' });
    expect(d.badge).toBe('TERM');
    expect(d.spoken).toBe('killed by SIGTERM');
    expect(d.badge).not.toBe('143');
    expect(d.ok).toBe(false);
  });

  it('keeps the full signal name where there is room for it', () => {
    // The badge is abbreviated because a tab is narrow; the accessible name is not, because
    // nothing is gained by abbreviating for a screen reader.
    expect(describeEnding({ exitCode: null, signal: 'SIGKILL' }).spoken).toBe('killed by SIGKILL');
  });

  it('refuses to call a contradictory ending clean', () => {
    // §4.8 says exactly one field is present. Both is a contradiction, and the one answer a
    // developer must be able to trust is that a green tick means the thing worked.
    const d = describeEnding({ exitCode: 0, signal: 'SIGTERM' });
    expect(d.ok).toBe(false);
    expect(d.badge).not.toBe('0');
  });

  it('refuses to call an empty ending clean', () => {
    // Neither field says nothing at all, and `Exited 0` would be inventing the answer.
    const d = describeEnding({ exitCode: null, signal: null });
    expect(d.ok).toBe(false);
    expect(d.spoken).toContain('unknown');
  });

  it('treats an empty signal string as no signal', () => {
    // A serialiser that writes `""` rather than omitting the field must not read as a kill.
    const d = describeEnding({ exitCode: 0, signal: '' });
    expect(d.ok).toBe(true);
    expect(d.badge).toBe('0');
  });
});
