/// What the last reconciliation did, per file, and how the status bar says it (FR-024).

export type ReconcileOutcome = 'fastForwarded' | 'merged' | 'conflicted' | 'notAttempted' | 'failed';

export interface ReconciledFile {
  relativePath: string;
  outcome: ReconcileOutcome;
  /// Why, for `failed`; absent otherwise.
  detail?: string | null;
}

export interface Reconciliation {
  /// Increases with every run the core records, so a dismissed report is not shown again while a
  /// new one is.
  run: number;
  files: ReconciledFile[];
}

const PHRASE: Record<ReconcileOutcome, string> = {
  fastForwarded: 'written',
  merged: 'merged with host changes',
  conflicted: 'needs your decision',
  notAttempted: 'not attempted, will retry',
  failed: 'failed',
};

/// One line for the bar, and one line per file for its title and accessible name.
///
/// Fast-forwards and merges are counted together in the summary -- both landed, and the bar's job
/// is to say whether anything needs attention -- but kept apart per file, because a merge is worth
/// reviewing and a fast-forward is not (the reason `Outcome` separates them). Attention first:
/// a summary that led with "3 reconciled" would bury the one conflict behind it.
export function describeReconciliation(r: Reconciliation): { summary: string; detail: string } {
  const count = (...o: ReconcileOutcome[]) => r.files.filter((f) => o.includes(f.outcome)).length;
  const parts: string[] = [];
  const conflicted = count('conflicted');
  const failed = count('failed');
  const waiting = count('notAttempted');
  const landed = count('fastForwarded', 'merged');
  if (conflicted) parts.push(`${conflicted} conflict${conflicted === 1 ? '' : 's'}`);
  if (failed) parts.push(`${failed} failed`);
  if (waiting) parts.push(`${waiting} not attempted`);
  if (landed) parts.push(`${landed} reconciled`);
  const detail = r.files
    .map((f) => {
      const why = f.outcome === 'failed' && f.detail ? `: ${f.detail}` : '';
      return `${f.relativePath || 'workspace'} — ${PHRASE[f.outcome]}${why}`;
    })
    .join('\n');
  return { summary: parts.join(' · '), detail };
}

/// Whether anything in the report is for the developer to act on or wait for.
export function needsAttention(r: Reconciliation): boolean {
  return r.files.some((f) => f.outcome !== 'fastForwarded' && f.outcome !== 'merged');
}
