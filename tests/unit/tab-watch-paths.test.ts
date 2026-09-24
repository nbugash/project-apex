import { describe, expect, it, vi } from 'vitest';
import { delta, watchedPaths, WatchRequester } from '../../client/ui/lib/workspace/watched.svelte';

/// The client sends reasons, not conclusions (FR-003c, FR-004, A-WATCHSCOPE).

describe('the watched set', () => {
  it('includes an open file whose folder is not expanded', () => {
    // Opened through search, or expanded, opened and collapsed again. FR-023 promises that
    // file is reported however the tree happens to be arranged.
    const paths = watchedPaths([], ['/src/deep/found.rs']);
    expect(paths).toEqual(['/src/deep/found.rs']);
  });

  it('removes a path when the last tab on it closes', () => {
    const before = watchedPaths(['/src'], ['/src/a.rs']);
    const after = watchedPaths(['/src'], []);
    expect(delta(before, after)).toEqual({ add: [], remove: ['/src/a.rs'] });
  });

  it('keeps the folder when a tab inside it is still open', () => {
    const before = watchedPaths(['/src'], ['/src/a.rs']);
    const after = watchedPaths([], ['/src/a.rs']);
    // The folder collapsed; the file is still open. The engine derives the directory from the
    // file path, so it stays watched — which is why the client sends both kinds.
    expect(delta(before, after)).toEqual({ add: [], remove: ['/src'] });
    expect(after).toContain('/src/a.rs');
  });

  it('sends a path once when it is both expanded and open', () => {
    expect(watchedPaths(['/src'], ['/src'])).toEqual(['/src']);
  });
});

describe('the requester', () => {
  it('coalesces rapid changes into one request', async () => {
    vi.useFakeTimers();
    const sent: unknown[] = [];
    const r = new WatchRequester((d) => sent.push(d), 50);
    r.update(['/a']);
    r.update(['/a', '/b']);
    r.update(['/a', '/b', '/c']);
    expect(sent).toHaveLength(0);
    vi.advanceTimersByTime(60);
    expect(sent).toHaveLength(1);
    vi.useRealTimers();
  });

  it('sends nothing when the set has not changed', () => {
    vi.useFakeTimers();
    const sent: unknown[] = [];
    const r = new WatchRequester((d) => sent.push(d), 50);
    r.update(['/a']);
    vi.advanceTimersByTime(60);
    r.update(['/a']);
    vi.advanceTimersByTime(60);
    expect(sent).toHaveLength(1);
    vi.useRealTimers();
  });

  it('re-sends the whole set on reconnection', () => {
    // Watches do not survive a dropped connection, and a client that resumed believing it was
    // still being told about changes would show a tree that had quietly stopped updating.
    vi.useFakeTimers();
    const sent: { add: string[] }[] = [];
    const r = new WatchRequester((d) => sent.push(d), 0);
    r.update(['/a', '/b']);
    vi.advanceTimersByTime(10);
    sent.length = 0;
    r.reestablish();
    expect(sent).toEqual([{ add: ['/a', '/b'], remove: [] }]);
    vi.useRealTimers();
  });
});
