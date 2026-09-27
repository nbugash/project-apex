// T053 — US3. Coordinates become decorations, a deletion becomes a position rather than a
// zero-length range, and an empty diff produces no decorations (FR-020, FR-023).
import { describe, it, expect } from 'vitest';
import { gutterMarks, emptyDiff } from '../../client/ui/lib/git/gutter';

describe('the editor gutter', () => {
  it('produces nothing for a file with no changes', () => {
    // The ordinary case. A gutter that drew something for an unchanged file would make every
    // file look modified, which is the same as marking none of them.
    expect(gutterMarks(emptyDiff())).toEqual([]);
    expect(gutterMarks(null)).toEqual([]);
    expect(gutterMarks(undefined)).toEqual([]);
  });

  it('marks an added range across all of its lines', () => {
    const marks = gutterMarks({ added: [[5, 7]], modified: [], deleted: [] });
    expect(marks).toHaveLength(1);
    expect(marks[0]).toMatchObject({ startLine: 5, endLine: 7, kind: 'added' });
  });

  it('marks a deletion as a single line rather than a zero-length range', () => {
    // **The case this module exists for.** The removed lines are not in the file, so there is
    // no range to draw; expressed as `[6, 5]` or `[6, 6)` it renders as nothing, which is
    // indistinguishable from not reporting the deletion at all.
    const marks = gutterMarks({ added: [], modified: [], deleted: [6] });
    expect(marks).toHaveLength(1);
    expect(marks[0]!.startLine).toBe(6);
    expect(marks[0]!.endLine).toBe(6);
    expect(marks[0]!.endLine).toBeGreaterThanOrEqual(marks[0]!.startLine);
  });

  it('keeps the three kinds distinguishable by class, not by colour', () => {
    const marks = gutterMarks({ added: [[1, 1]], modified: [[2, 2]], deleted: [3] });
    const classes = marks.map((m) => m.className);
    expect(new Set(classes).size).toBe(3);
    for (const c of classes) expect(c).not.toMatch(/#[0-9a-f]{3,8}/i);
  });

  it('returns marks in line order', () => {
    // git prints hunks in file order but the three lists arrive separately, so without this a
    // reader comparing two runs has to think about which list happened to be built first.
    const marks = gutterMarks({
      added: [[10, 11]],
      modified: [[2, 3]],
      deleted: [7],
    });
    expect(marks.map((m) => m.startLine)).toEqual([2, 7, 10]);
  });

  it('drops a range the engine could not have meant rather than clamping it', () => {
    // Clamping invents a mark on a line nothing said anything about, and the developer has no
    // way to tell an invented mark from a real one.
    const marks = gutterMarks({
      added: [
        [0, 3],
        [5, 2],
      ],
      modified: [[-1, -1]],
      deleted: [0, -4],
    });
    expect(marks).toEqual([]);
  });

  it('handles a one-line change, which is the commonest change there is', () => {
    const marks = gutterMarks({ added: [], modified: [[42, 42]], deleted: [] });
    expect(marks).toEqual([
      { startLine: 42, endLine: 42, kind: 'modified', className: 'vk-gutter-modified' },
    ]);
  });
});
