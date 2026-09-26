/// The rules a buffer keeps, which are the rules that stop work being lost.
///
/// Each case here is one that a plausible implementation gets wrong. The dirty-flag cases in
/// particular: clearing it when the request goes out satisfies every visible behaviour and tells
/// the developer their work is on the host when it is in flight, or gone (FR-013).
import { describe, expect, it } from 'vitest';
import { Buffer, BufferSet } from '../../client/ui/lib/editor/buffers.svelte';

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
    const a = set.open('src/main.rs');
    const b = set.open('src/main.rs');
    expect(a).toBe(b);
    expect(set.buffers.length).toBe(1);
  });

  it('keeps different files apart', () => {
    const set = new BufferSet();
    set.open('a.rs');
    set.open('b.rs');
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
