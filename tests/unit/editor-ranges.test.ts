/// Which bytes of a large file the buffer holds.
///
/// Pure arithmetic, deliberately: it decides what to fetch, and getting it wrong means either
/// refetching what is already held or rendering a gap as content. Neither needs a window to
/// find, so neither should need one to test.
import { describe, expect, it } from 'vitest';
import { LoadedRegions } from '../../client/ui/lib/editor/ranges';

describe('loaded regions', () => {
  it('holds nothing before anything is read', () => {
    const r = new LoadedRegions(1000);
    expect(r.complete()).toBe(false);
    expect(r.covers(0, 1)).toBe(false);
  });

  it('is complete once the whole file is covered', () => {
    const r = new LoadedRegions(100);
    r.add(0, 100);
    expect(r.complete()).toBe(true);
  });

  it('merges adjacent ranges rather than accumulating them', () => {
    // Left unmerged, `missingFor` would walk a list that grows with every scroll, and a file
    // read in a hundred steps would answer a hundred times slower than one read whole.
    const r = new LoadedRegions(300);
    r.add(0, 100);
    r.add(100, 200);
    expect(r.spans()).toEqual([[0, 200]]);
  });

  it('merges overlapping ranges', () => {
    const r = new LoadedRegions(300);
    r.add(0, 150);
    r.add(100, 200);
    expect(r.spans()).toEqual([[0, 200]]);
  });

  it('keeps disjoint ranges apart and ascending', () => {
    const r = new LoadedRegions(300);
    r.add(200, 300);
    r.add(0, 100);
    expect(r.spans()).toEqual([
      [0, 100],
      [200, 300],
    ]);
    expect(r.complete()).toBe(false);
  });

  it('asks only for what is absent', () => {
    // The point of the whole module: a scroll into a partly-held region must not refetch the
    // part already held.
    const r = new LoadedRegions(1000);
    r.add(100, 200);
    expect(r.missingFor(150, 400)).toEqual([[200, 400]]);
  });

  it('splits a request around a hole it already spans', () => {
    const r = new LoadedRegions(1000);
    r.add(200, 300);
    expect(r.missingFor(100, 400)).toEqual([
      [100, 200],
      [300, 400],
    ]);
  });

  it('asks for nothing when the range is already held', () => {
    const r = new LoadedRegions(1000);
    r.add(0, 500);
    expect(r.missingFor(100, 200)).toEqual([]);
  });

  it('narrows rather than corrupts when the file shrinks', () => {
    // A range response shorter than requested is not an error: the file may have shrunk between
    // requests. Recording what arrived and re-reading the total is the honest response;
    // recording what was asked for would claim bytes that are not held.
    const r = new LoadedRegions(1000);
    r.add(0, 1000);
    r.resize(400);
    expect(r.spans()).toEqual([[0, 400]]);
    expect(r.complete()).toBe(true);
  });

  it('is complete for an empty file', () => {
    // Zero-length files exist, and a `complete()` that returned false for one would leave the
    // buffer permanently read-only.
    expect(new LoadedRegions(0).complete()).toBe(true);
  });
});
