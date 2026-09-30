import { describe, expect, it } from 'vitest';
import {
  describeReconciliation,
  needsAttention,
  type Reconciliation,
} from '../../client/ui/lib/offline/reconciliation';

const report = (...outcomes: Reconciliation['files'][number]['outcome'][]): Reconciliation => ({
  run: 1,
  files: outcomes.map((outcome, i) => ({ relativePath: `/f${i}.rs`, outcome })),
});

describe('reconciliation report on the status bar (FR-024)', () => {
  it('counts what landed together, and leads with what needs attention', () => {
    const { summary } = describeReconciliation(
      report('fastForwarded', 'merged', 'conflicted', 'notAttempted', 'failed'),
    );
    expect(summary).toBe('1 conflict · 1 failed · 1 not attempted · 2 reconciled');
  });

  it('says per file what happened, keeping a merge apart from a fast-forward', () => {
    const { detail } = describeReconciliation({
      run: 1,
      files: [
        { relativePath: '/a.rs', outcome: 'fastForwarded' },
        { relativePath: '/b.rs', outcome: 'merged' },
        { relativePath: '', outcome: 'failed', detail: 'the root is gone' },
      ],
    });
    expect(detail.split('\n')).toEqual([
      '/a.rs — written',
      '/b.rs — merged with host changes',
      'workspace — failed: the root is gone',
    ]);
  });

  it('needs attention only when something did not land', () => {
    expect(needsAttention(report('fastForwarded', 'merged'))).toBe(false);
    expect(needsAttention(report('merged', 'notAttempted'))).toBe(true);
  });
});
