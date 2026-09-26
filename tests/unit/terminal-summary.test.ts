/// FR-032: what the developer is told after a reconnection.
///
/// The count comes from `attach`'s `retained`, never from a per-frame marker. A replayed frame is
/// deliberately indistinguishable from a live one, so a flag saying "this part is old" would
/// exist only to be ignored -- or to be branched on and draw a seam that is not in the build's
/// output.
import { describe, expect, it } from 'vitest';
import {
  summarise,
  Terminals,
  type ReconnectionSummary,
} from '../../client/ui/lib/terminal/terminals.svelte';

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

describe('the reconnection summary (FR-032)', () => {
  it('says nothing when there is nothing to say', () => {
    // Different from reassuring somebody about work that does not exist. A client with no tasks
    // reconnects all the time, and a line every time would train them to ignore the line.
    expect(summarise({ outcomes: [], retained: 0 })).toBeNull();
  });

  it('reports how each finished task finished, not how many did', () => {
    // "2 finished" is a line that makes somebody go and look, and the endings are already known.
    const summary: ReconnectionSummary = {
      outcomes: [
        { kind: 'finished', taskId: 'build', ending: 'exited 0' },
        { kind: 'finished', taskId: 'test', ending: 'exited 101' },
      ],
      retained: 0,
    };
    const line = summarise(summary);
    expect(line).toContain('build exited 0');
    expect(line).toContain('test exited 101');
  });

  it('counts what survived and what is gone separately', () => {
    // They lead to different actions: one is a build to go back to, the other is a build to
    // start again.
    const line = summarise({
      outcomes: [
        { kind: 'survived', taskId: 'a', retained: 0 },
        { kind: 'survived', taskId: 'b', retained: 0 },
        { kind: 'gone', taskId: 'c' },
      ],
      retained: 0,
    });
    expect(line).toContain('2 still running');
    expect(line).toContain('1 no longer reachable');
  });

  it('reports what was missed in units a person reads', () => {
    // Whole units only: a developer wants to know whether they missed a line or a megabyte, and
    // the digits after the point answer neither question.
    const one = (retained: number) =>
      summarise({
        outcomes: [{ kind: 'survived', taskId: 'a', retained }],
        retained,
      });
    expect(one(512)).toContain('512 B missed');
    expect(one(4096)).toContain('4 KB missed');
    expect(one(5 * 1024 * 1024)).toContain('5 MB missed');
  });

  it('says nothing about bytes when none were missed', () => {
    // A reconnection that missed nothing should not mention it. "0 B missed" is noise that makes
    // the line longer and tells the reader nothing.
    const line = summarise({
      outcomes: [{ kind: 'survived', taskId: 'a', retained: 0 }],
      retained: 0,
    });
    expect(line).not.toContain('missed');
  });

  it('writes the summary into the panel the developer is looking at', () => {
    // A build's transcript is the right place for a note about that build, and it is where their
    // attention already is.
    const terminals = new Terminals();
    const panel = terminals.show('build');
    const line = terminals.reconnected({
      outcomes: [{ kind: 'survived', taskId: 'build', retained: 2048 }],
      retained: 2048,
    });

    expect(line).toContain('2 KB missed');
    const written = text(panel.buffered());
    expect(written).toContain('[reconnected]');
    expect(written).toContain('2 KB missed');
    // Carriage return with the newline: a terminal's cursor does not return on its own, so a
    // bare \n leaves the next line indented by however long the last one was.
    expect(written).toContain('\r\n');
  });

  it('leaves the panels themselves alone', () => {
    // A task that survived keeps its scrollback, and one that finished keeps the output that led
    // to its ending. The summary is added to a transcript, not substituted for one.
    const terminals = new Terminals();
    const panel = terminals.show('build');
    panel.write('compiling\r\n');
    terminals.reconnected({
      outcomes: [{ kind: 'survived', taskId: 'build', retained: 0 }],
      retained: 0,
    });
    expect(text(panel.buffered())).toContain('compiling');
  });

  it('writes nothing when there is nothing to report', () => {
    const terminals = new Terminals();
    const panel = terminals.show('build');
    expect(terminals.reconnected({ outcomes: [], retained: 0 })).toBeNull();
    expect(panel.buffered().length).toBe(0);
  });
});
