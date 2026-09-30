/// How a conflict's three sides and its reason are put into words (US4, FR-021).
///
/// Pure, and separate from the panel, so what the developer is told can be tested without
/// rendering: each side is labelled in words and marked with its own icon, so the three are
/// distinguishable without colour, and a side that does not exist says why rather than rendering
/// as an empty box that reads as "empty file".

export type ConflictReason = 'overlap' | 'notText' | 'deletedOnHost' | 'createdOnBothSides';

export interface Conflict {
  relativePath: string;
  base: string | null;
  local: string | null;
  remote: string | null;
  basePresent: boolean;
  remotePresent: boolean;
  remoteSha256: string | null;
  mergeable: boolean;
  reason: ConflictReason;
  draft: string | null;
}

export type Side = 'base' | 'local' | 'remote';

export const SIDES: readonly { side: Side; label: string; icon: string }[] = [
  { side: 'base', label: 'Started from', icon: 'ph-git-commit' },
  { side: 'local', label: 'Yours, saved offline', icon: 'ph-user' },
  { side: 'remote', label: 'On the host now', icon: 'ph-cloud' },
];

export const REASON: Record<ConflictReason, string> = {
  overlap: 'You and the host changed the same lines. Edit the result below, or keep one side.',
  notText: 'This file is not text, so it cannot be combined. Keep one side.',
  deletedOnHost: 'The host deleted this file while you were editing it offline.',
  createdOnBothSides: 'You created this file offline, and the host now has a file at the same path.',
};

/// A side's text, or the sentence that stands in for it.
export function sideContent(c: Conflict, side: Side): { text: string } | { absent: string } {
  const text = c[side];
  if (text !== null) return { text };
  if (side === 'base' && !c.basePresent) return { absent: 'Created offline: there was no earlier version.' };
  if (side === 'remote' && !c.remotePresent) return { absent: 'Deleted on the host.' };
  return { absent: 'Not text, so it cannot be shown here.' };
}

/// The two whole-file choices, worded for what they do to this particular file.
export function wholeFileChoices(c: Conflict): { keepLocal: string; takeRemote: string } {
  return {
    keepLocal: c.remotePresent ? 'Keep yours' : 'Keep yours (recreate the file)',
    takeRemote: c.remotePresent ? "Take the host's" : 'Accept the deletion',
  };
}
