/// The rules a buffer keeps, which are the rules that stop work being lost.
///
/// Each case here is one that a plausible implementation gets wrong. The dirty-flag cases in
/// particular: clearing it when the request goes out satisfies every visible behaviour and tells
/// the developer their work is on the host when it is in flight, or gone (FR-013).
import { afterEach, describe, expect, it } from 'vitest';
import {
  Buffer,
  BufferSet,
  MAX_TEXT_FILE,
  load,
  loadRest,
  onFileEvent,
  save,
} from '../../client/ui/lib/editor/buffers.svelte';
import { ReadFailed, setEditorSink, type Chunk, type EditorSink } from '../../client/ui/lib/editor/sink';
import type { WriteOutcome } from '../../client/ui/lib/editor/buffers.svelte';

const filled = (text = 'hello', sha = 'aaa', total = 5) => {
  const b = new Buffer('src/main.rs');
  b.fill(text, sha, total);
  return b;
};

describe('one buffer per file', () => {
  it('returns the same buffer however many times a path is opened', () => {
    // Two buffers of one file means two bases, and a save through one silently reverts the
    // other (FR-023, SC-010).
    const set = new BufferSet();
    const a = set.ensure('src/main.rs');
    const b = set.ensure('src/main.rs');
    expect(a).toBe(b);
    expect(set.buffers.length).toBe(1);
  });

  it('keeps different files apart', () => {
    const set = new BufferSet();
    set.ensure('a.rs');
    set.ensure('b.rs');
    expect(set.buffers.length).toBe(2);
  });
});

describe('the dirty flag', () => {
  it('is not set by filling from the host', () => {
    expect(filled().dirty).toBe(false);
  });

  it('is set by an edit', () => {
    const b = filled();
    b.applyEdit('hello!');
    expect(b.dirty).toBe(true);
  });

  it('is cleared only by a write that landed', () => {
    const b = filled();
    b.applyEdit('hello!');
    b.adopt('bbb');
    expect(b.dirty).toBe(false);
    expect(b.base).toBe('bbb');
  });

  it('survives a conflict, with the text untouched', () => {
    // FR-013 and FR-011. The developer's work is still theirs and still unsaved.
    const b = filled();
    b.applyEdit('mine');
    b.failed({ kind: 'conflict' });
    expect(b.dirty).toBe(true);
    expect(b.text).toBe('mine');
    expect(b.ending).toEqual({ kind: 'conflict' });
  });

  it('survives an unreachable engine, with the text untouched', () => {
    const b = filled();
    b.applyEdit('mine');
    b.failed({ kind: 'unreachable' });
    expect(b.dirty).toBe(true);
    expect(b.text).toBe('mine');
  });

  it('is cleared by an explicit reload, which is the developer discarding', () => {
    const b = filled();
    b.applyEdit('mine');
    b.reload('theirs', 'ccc', 6);
    expect(b.dirty).toBe(false);
    expect(b.text).toBe('theirs');
    expect(b.base).toBe('ccc');
  });
});

describe('a partially loaded buffer', () => {
  it('is not editable', () => {
    // A whole-file write of a partial buffer replaces the unloaded regions with nothing, and the
    // protocol carries content rather than a patch, so there is no safe partial write.
    const b = new Buffer('big.log');
    b.fill('first page', 'aaa', 10_000, [0, 10]);
    expect(b.editable).toBe(false);
  });

  it('refuses an edit rather than accepting one it would discard', () => {
    const b = new Buffer('big.log');
    b.fill('first page', 'aaa', 10_000, [0, 10]);
    expect(b.applyEdit('tampered')).toBe(false);
    expect(b.text).toBe('first page');
    expect(b.dirty).toBe(false);
  });

  it('becomes editable once every byte is held', () => {
    const b = new Buffer('big.log');
    b.fill('first', 'aaa', 10, [0, 5]);
    expect(b.editable).toBe(false);
    b.loaded.add(5, 10);
    expect(b.editable).toBe(true);
  });
});

describe('the base hash', () => {
  it('is absent until the buffer is filled', () => {
    expect(new Buffer('x.rs').base).toBeNull();
  });

  it('is never changed by an edit', () => {
    // The base is what the developer started from. Adopting a newer hash here would make a save
    // overwrite whatever arrived in the meantime, which is the whole failure the check prevents.
    const b = filled('hello', 'aaa');
    b.applyEdit('hello!');
    expect(b.base).toBe('aaa');
  });
});

// ---- Reading, saving, and hearing about a file that moved ----

/// A sink that answers from a script and records what actually left.
///
/// Counting here rather than trusting the caller: SC-002's claim is about requests issued, and
/// a test that counted intentions would pass for an implementation that issued two.
function stubSink(over: Partial<EditorSink>): { sink: EditorSink; calls: string[] } {
  const calls: string[] = [];
  const sink: EditorSink = {
    async read(_path, range) {
      calls.push(range ? `readRange:${range[0]}-${range[1]}` : 'read');
      throw new ReadFailed({ kind: 'other', message: 'no read scripted' });
    },
    async write() {
      calls.push('write');
      return { kind: 'written', sha256: 'b'.repeat(64) };
    },
    async hash() {
      calls.push('hash');
      return null;
    },
    ...over,
  };
  return { sink, calls };
}

const chunk = (text: string, sha: string, total = text.length, offset = 0): Chunk => ({
  text,
  sha256: sha,
  total,
  offset,
});

let restore: EditorSink | null = null;
afterEach(() => {
  if (restore) setEditorSink(restore);
  restore = null;
});
function use(sink: EditorSink): void {
  restore = setEditorSink(sink);
}

describe('opening a file', () => {
  it('reads a small file whole, in one request', async () => {
    // FR-018. A range request for a file that fits costs the same round trip and delivers less.
    const { sink, calls } = stubSink({
      async read(_p, range) {
        calls.push(range ? 'readRange' : 'read');
        return chunk('fn main() {}', 'a'.repeat(64));
      },
    });
    use(sink);
    const set = new BufferSet();
    const b = await set.open('src/main.rs');
    expect(b.text).toBe('fn main() {}');
    expect(b.base).toBe('a'.repeat(64));
    expect(b.editable).toBe(true);
    expect(calls.filter((c) => c.startsWith('read')).length).toBe(1);
  });

  it('does not re-read a file that is already open', async () => {
    // A reopen that refetched would discard unsaved work every time a developer clicked a tab
    // they already had open (FR-023, SC-010).
    let reads = 0;
    use(
      stubSink({
        async read() {
          reads += 1;
          return chunk('x', 'a'.repeat(64));
        },
      }).sink,
    );
    const set = new BufferSet();
    const first = await set.open('a.rs');
    first.applyEdit('x + unsaved');
    const second = await set.open('a.rs');
    expect(second).toBe(first);
    expect(second.text).toBe('x + unsaved');
    expect(reads).toBe(1);
  });

  it('declines content that is not text, and names what will render it', async () => {
    use(
      stubSink({
        async read() {
          throw new ReadFailed({ kind: 'notText', message: 'not text' });
        },
      }).sink,
    );
    const b = new Buffer('logo.png');
    await load(b);
    expect(b.notice).toEqual({ kind: 'binary' });
    expect(b.text).toBe('');
  });

  it('declines a file larger than it will open, and says how large', async () => {
    // "Too large" without the size tells a developer nothing they can act on.
    const total = MAX_TEXT_FILE + 1;
    use(
      stubSink({
        async read() {
          throw new ReadFailed({ kind: 'tooLarge', total, message: 'too large' });
        },
      }).sink,
    );
    const b = new Buffer('huge.log');
    await load(b);
    expect(b.notice).toEqual({ kind: 'tooLarge', total, limit: MAX_TEXT_FILE });
  });

  it('opens a large file as a window, and refuses to edit it until the rest is held', async () => {
    // A whole-file write of a partial buffer replaces the unloaded regions with nothing, and
    // §4.8 carries content rather than a patch (research.md, *Ranges*).
    const total = 900 * 1024;
    use(
      stubSink({
        async read(_p, range) {
          if (!range) throw new ReadFailed({ kind: 'tooLarge', total, message: 'too large' });
          return chunk('x'.repeat(range[1] - range[0]), 'a'.repeat(64), total, range[0]);
        },
      }).sink,
    );
    const b = new Buffer('big.log');
    await load(b);
    expect(b.loaded.total).toBe(total);
    expect(b.editable).toBe(false);
    expect(b.applyEdit('nope')).toBe(false);
  });

  it('records regions in bytes rather than in code units', async () => {
    // `text.length` counts UTF-16 code units, so one emoji counts as two and every region after
    // it is wrong. The error shows up much later as a range that never fills.
    const text = 'a😀b';
    const bytes = new TextEncoder().encode(text).length;
    expect(bytes).not.toBe(text.length);
    use(
      stubSink({
        async read() {
          return chunk(text, 'a'.repeat(64), bytes);
        },
      }).sink,
    );
    const b = new Buffer('emoji.txt');
    await load(b);
    expect(b.editable).toBe(true);
  });
});

describe('saving', () => {
  it('will not start a second write while one is in flight', async () => {
    // FR-014. Two writes together each carry the base the buffer held when they started, and
    // whichever lands second is refused for a conflict the developer caused by saving twice.
    let writes = 0;
    let release: (() => void) | null = null;
    const blocked = new Promise<void>((r) => {
      release = r;
    });
    use(
      stubSink({
        async write() {
          writes += 1;
          await blocked;
          return { kind: 'written', sha256: 'b'.repeat(64) };
        },
      }).sink,
    );
    const b = filled();
    b.applyEdit('changed');

    const first = save(b);
    const second = await save(b);
    expect(second).toBe(null);
    expect(writes).toBe(1);

    release!();
    await first;
    expect(writes).toBe(1);
  });

  it('keeps the buffer dirty while the write is outstanding', async () => {
    // The failing case is an optimistic clear: marking the buffer saved when the request goes
    // out satisfies every other behaviour and tells the developer their work is on the host
    // when it is in flight, or lost (FR-013).
    let release: (() => void) | null = null;
    const blocked = new Promise<void>((r) => {
      release = r;
    });
    use(
      stubSink({
        async write() {
          await blocked;
          return { kind: 'written', sha256: 'b'.repeat(64) };
        },
      }).sink,
    );
    const b = filled();
    b.applyEdit('changed');

    const pending = save(b);
    expect(b.dirty).toBe(true);
    expect(b.saving).toBe(true);

    release!();
    await pending;
    expect(b.dirty).toBe(false);
    expect(b.base).toBe('b'.repeat(64));
  });

  it('leaves the text untouched and the buffer dirty when the engine cannot be reached', async () => {
    use(
      stubSink({
        async write(): Promise<WriteOutcome> {
          return { kind: 'unreachable' };
        },
      }).sink,
    );
    const b = filled();
    b.applyEdit('work in progress');
    await save(b);
    expect(b.text).toBe('work in progress');
    expect(b.dirty).toBe(true);
    expect(b.ending).toEqual({ kind: 'unreachable' });
  });

  it('refuses to save a partially loaded buffer', async () => {
    let writes = 0;
    use(
      stubSink({
        async write() {
          writes += 1;
          return { kind: 'written', sha256: 'b'.repeat(64) };
        },
      }).sink,
    );
    const b = new Buffer('big.log');
    b.fill('first window', 'a'.repeat(64), 900 * 1024, [0, 12]);
    b.dirty = true;
    expect(await save(b)).toBe(null);
    expect(writes).toBe(0);
  });
});

describe('a file that moved on the host', () => {
  it('says nothing when the change is our own write', async () => {
    // A-WRITEECHO. A save produces an event, and reporting "changed on the host" for the
    // developer's own write is the noise that makes every later notice ignorable (SC-012).
    const sha = 'a'.repeat(64);
    use(
      stubSink({
        async hash() {
          return sha;
        },
      }).sink,
    );
    const b = filled('hello', sha);
    await onFileEvent(b, 'changed');
    expect(b.notice).toBe(null);
  });

  it('reports a genuine change against a dirty buffer without replacing it', async () => {
    // Replacing it discards work the developer has not saved and cannot recover (FR-024b).
    use(
      stubSink({
        async hash() {
          return 'c'.repeat(64);
        },
      }).sink,
    );
    const b = filled('hello', 'a'.repeat(64));
    b.applyEdit('my unsaved work');
    await onFileEvent(b, 'changed');
    expect(b.notice).toEqual({ kind: 'diverged' });
    expect(b.text).toBe('my unsaved work');
    expect(b.dirty).toBe(true);
  });

  it('refreshes a clean buffer from the host', async () => {
    use(
      stubSink({
        async hash() {
          return 'c'.repeat(64);
        },
        async read() {
          return chunk('theirs', 'c'.repeat(64));
        },
      }).sink,
    );
    const b = filled('hello', 'a'.repeat(64));
    await onFileEvent(b, 'changed');
    expect(b.text).toBe('theirs');
    expect(b.base).toBe('c'.repeat(64));
    expect(b.dirty).toBe(false);
  });

  it('reports a deletion without discarding the buffer', async () => {
    // FR-025. Discarding it would destroy unsaved work because somebody else removed the file,
    // and the developer may well want to save it back.
    use(stubSink({}).sink);
    const b = filled('hello', 'a'.repeat(64));
    b.applyEdit('work that only exists here');
    await onFileEvent(b, 'deleted');
    expect(b.notice).toEqual({ kind: 'missing' });
    expect(b.text).toBe('work that only exists here');
  });
});

describe('finishing a partial buffer', () => {
  it('loads what is missing so the buffer becomes editable', async () => {
    const total = 700 * 1024;
    use(
      stubSink({
        async read(_p, range) {
          if (!range) throw new ReadFailed({ kind: 'tooLarge', total, message: 'too large' });
          return chunk('x'.repeat(range[1] - range[0]), 'a'.repeat(64), total, range[0]);
        },
      }).sink,
    );
    const b = new Buffer('big.log');
    await load(b);
    expect(b.editable).toBe(false);
    await loadRest(b);
    expect(b.editable).toBe(true);
  });

  it('stops rather than spinning when the host stops making progress', async () => {
    // A host answering with nothing would turn "make this editable" into a hang, which is a
    // worse outcome than saying it could not be done.
    const total = 700 * 1024;
    let reads = 0;
    use(
      stubSink({
        async read(_p, range) {
          reads += 1;
          if (!range) throw new ReadFailed({ kind: 'tooLarge', total, message: 'too large' });
          return chunk(reads === 1 ? 'x'.repeat(1000) : '', 'a'.repeat(64), total, range[0]);
        },
      }).sink,
    );
    const b = new Buffer('big.log');
    await load(b);
    await loadRest(b);
    expect(b.editable).toBe(false);
    expect(reads).toBeLessThan(10);
  });
});

describe('a tab restored from a previous session', () => {
  it('reports a file that no longer exists rather than showing an empty document', async () => {
    // FR-022. An empty editor looks like a file that is there and happens to be empty, and
    // saving it would create one -- which is how a restored tab silently truncates a file that
    // somebody moved between sessions.
    use(
      stubSink({
        async read() {
          throw new ReadFailed({ kind: 'notFound', message: 'no such file' });
        },
      }).sink,
    );
    const set = new BufferSet();
    const b = await set.open('src/gone.rs');
    expect(b.notice).toEqual({ kind: 'missing' });
    expect(b.base).toBe(null);
  });
});
