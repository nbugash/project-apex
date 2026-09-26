/**
 * Which byte ranges of a file the buffer currently holds.
 *
 * Pure: no Monaco, no IPC, no clock. It decides what to fetch, and that decision is arithmetic —
 * fetching what is already held wastes a round trip, and believing a gap is held renders nothing
 * as content. Neither needs a window to find.
 */

/** Half-open `[start, end)`, as every range in this feature is. */
export type ByteRange = [number, number];

export class LoadedRegions {
  /// The file's size as last reported. Not `const`: a file can shrink between reads.
  #total: number;
  /// Non-overlapping, ascending, merged on insert. The invariant every method relies on.
  #spans: ByteRange[] = [];

  constructor(total: number) {
    this.#total = Math.max(0, total);
  }

  get total(): number {
    return this.#total;
  }

  spans(): ByteRange[] {
    return this.#spans.map(([s, e]) => [s, e] as ByteRange);
  }

  /// Record bytes that have arrived, merging with whatever touches them.
  ///
  /// Merging on insert rather than on read is what keeps `missingFor` linear in the number of
  /// holes rather than in the number of reads: a file scrolled through in a hundred steps would
  /// otherwise carry a hundred spans describing one contiguous region.
  add(start: number, end: number): void {
    const from = Math.max(0, Math.min(start, this.#total));
    const to = Math.max(from, Math.min(end, this.#total));
    if (to === from) return;

    const merged: ByteRange[] = [];
    let [lo, hi] = [from, to];
    for (const [s, e] of this.#spans) {
      // `e < lo` rather than `e <= lo`: touching spans merge, because `[0,100)` and `[100,200)`
      // together hold every byte in `[0,200)` and describing them separately would make
      // `missingFor` report a hole that does not exist.
      if (e < lo) merged.push([s, e]);
      else if (s > hi) {
        merged.push([lo, hi]);
        [lo, hi] = [s, e];
      } else {
        lo = Math.min(lo, s);
        hi = Math.max(hi, e);
      }
    }
    merged.push([lo, hi]);
    this.#spans = merged;
  }

  covers(start: number, end: number): boolean {
    return this.missingFor(start, end).length === 0;
  }

  /// Whether every byte of the file is held. An empty file is complete by definition — a
  /// `false` here would leave its buffer permanently read-only.
  complete(): boolean {
    if (this.#total === 0) return true;
    return this.#spans.length === 1 && this.#spans[0]![0] === 0 && this.#spans[0]![1] === this.#total;
  }

  /// The parts of `[start, end)` that are not held, in order.
  missingFor(start: number, end: number): ByteRange[] {
    const from = Math.max(0, Math.min(start, this.#total));
    const to = Math.max(from, Math.min(end, this.#total));
    const holes: ByteRange[] = [];
    let cursor = from;
    for (const [s, e] of this.#spans) {
      if (e <= cursor) continue;
      if (s >= to) break;
      if (s > cursor) holes.push([cursor, Math.min(s, to)]);
      cursor = Math.max(cursor, e);
      if (cursor >= to) break;
    }
    if (cursor < to) holes.push([cursor, to]);
    return holes;
  }

  /// The file turned out to be a different size.
  ///
  /// Held ranges are clipped rather than discarded: the bytes that are still within the file are
  /// still the bytes that were read. Discarding would refetch a whole file because its last line
  /// was deleted.
  resize(total: number): void {
    this.#total = Math.max(0, total);
    this.#spans = this.#spans
      .map(([s, e]) => [Math.min(s, this.#total), Math.min(e, this.#total)] as ByteRange)
      .filter(([s, e]) => e > s);
  }
}
