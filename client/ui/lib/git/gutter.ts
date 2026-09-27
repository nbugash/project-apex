/// Line coordinates to editor decorations.
///
/// Pure, so the mapping is testable without a window — and the mapping is where the one
/// genuinely awkward case lives. A **deletion is a position, not a range**: the removed lines
/// are not in the file any more, so there is nothing to draw a range over. Expressed as a
/// zero-length range it renders as nothing at all, which is the same as not reporting it.

/** What the engine sends: one-based, inclusive line coordinates (contracts/git-status.md). */
export interface FileDiff {
  /// Ranges present now and not before, as `[start, end]`.
  added: Array<[number, number]>;
  /// Ranges whose content differs.
  modified: Array<[number, number]>;
  /// Positions where lines were removed. The removed lines are not in this file.
  deleted: number[];
}

export type GutterKind = 'added' | 'modified' | 'deleted';

/// One mark, in the shape an editor wants: a range and a class.
///
/// `startLine` and `endLine` are one-based and inclusive, matching both the engine and what an
/// editor numbers its lines — so nothing converts, and nothing is off by one because somebody
/// converted twice.
export interface GutterMark {
  startLine: number;
  endLine: number;
  kind: GutterKind;
  /// The design-system class the gutter paints. Named, never a colour.
  className: string;
}

const CLASS: Record<GutterKind, string> = {
  added: 'vk-gutter-added',
  modified: 'vk-gutter-modified',
  deleted: 'vk-gutter-deleted',
};

export function emptyDiff(): FileDiff {
  return { added: [], modified: [], deleted: [] };
}

/// Decorations for one file's diff, in line order.
///
/// Sorted because an editor applies them in the order given and a reader comparing two runs
/// should not have to think about which hunk git happened to print first.
export function gutterMarks(diff: FileDiff | null | undefined): GutterMark[] {
  if (!diff) return [];
  const marks: GutterMark[] = [];

  for (const [start, end] of diff.added ?? []) {
    if (valid(start, end)) marks.push(mark(start, end, 'added'));
  }
  for (const [start, end] of diff.modified ?? []) {
    if (valid(start, end)) marks.push(mark(start, end, 'modified'));
  }
  for (const at of diff.deleted ?? []) {
    // **The line below the deletion**, marked as a single line. A deletion at line 6 means
    // "something used to be here", and the only place to say so is the line that now occupies
    // the position. A range would need an end, and there is no end to give.
    if (Number.isInteger(at) && at >= 1) marks.push(mark(at, at, 'deleted'));
  }

  return marks.sort((a, b) => a.startLine - b.startLine || a.kind.localeCompare(b.kind));
}

function valid(start: number, end: number): boolean {
  // A range the engine could not have meant is dropped rather than clamped. Clamping invents a
  // mark on a line nothing said anything about, and the developer has no way to know.
  return Number.isInteger(start) && Number.isInteger(end) && start >= 1 && end >= start;
}

function mark(startLine: number, endLine: number, kind: GutterKind): GutterMark {
  return { startLine, endLine, kind, className: CLASS[kind] };
}
