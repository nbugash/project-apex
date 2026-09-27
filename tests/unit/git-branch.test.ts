// T045 — US2. The three cases render as a name, a short commit, and nothing at all — never an
// empty label and never a placeholder (FR-019, SC-009).
import { describe, it, expect } from 'vitest';
import { branchLabel } from '../../client/ui/lib/git/branch';

describe('the branch indicator', () => {
  it('shows a branch by name', () => {
    const label = branchLabel({ kind: 'branch', name: 'feature/thing' });
    expect(label?.text).toBe('feature/thing');
    expect(label?.title).toContain('feature/thing');
  });

  it('shows a detached head as a short commit, not as a branch', () => {
    // The failure this guards is a reading of git's own output: `# branch.head (detached)` puts
    // the word where a name goes, and an indicator that trusted it would tell a developer
    // mid-rebase they are on a branch they cannot push.
    const label = branchLabel({ kind: 'detached', commit: '9f1c2ab3d4e5f6' });
    expect(label?.text).toBe('9f1c2ab');
    expect(label?.text).not.toContain('detached');
    expect(label?.icon).not.toBe(branchLabel({ kind: 'branch', name: 'x' })?.icon);
  });

  it('carries the whole commit where it can be read in full', () => {
    // Truncation is for the strip of space the status bar has, not for the fact.
    expect(branchLabel({ kind: 'detached', commit: '9f1c2ab3d4e5f6' })?.title).toContain(
      '9f1c2ab3d4e5f6',
    );
  });

  it('shows nothing at all when there is no repository', () => {
    // Not "no branch", not a dash. A developer not using git should see no statement about git
    // occupying the status bar.
    expect(branchLabel({ kind: 'none' })).toBeNull();
    expect(branchLabel(null)).toBeNull();
    expect(branchLabel(undefined)).toBeNull();
  });

  it('never produces an empty label', () => {
    // An empty label renders as a gap, which reads as a rendering fault rather than as an
    // absence. Every case either says something or is absent.
    const cases = [
      { kind: 'branch', name: '' },
      { kind: 'detached', commit: '' },
      { kind: 'none' },
    ] as const;
    for (const c of cases) {
      const label = branchLabel(c);
      expect(label === null || label.text.length > 0).toBe(true);
    }
  });

  it('says nothing for a case this build has not heard of', () => {
    // A newer core reporting a fourth kind must degrade to silence, not to the word itself.
    expect(branchLabel({ kind: 'rebasing' } as never)).toBeNull();
  });
});
