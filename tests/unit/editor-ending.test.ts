/// What a save's ending reads as.
///
/// Assertions on wording, which is unusual and is the point: FR-012 is a requirement about what
/// a person understands, and a test that only checked the variant would pass for an
/// implementation that rendered both as "save failed".
import { describe, expect, it } from 'vitest';
import { describeOutcome } from '../../client/ui/lib/editor/ending';
import type { WriteOutcome } from '../../client/ui/lib/editor/buffers.svelte';

const conflict = describeOutcome({ kind: 'conflict' });
const unreachable = describeOutcome({ kind: 'unreachable' });

describe('a conflict and a dropped link never read as each other', () => {
  it('names somebody else for a conflict', () => {
    expect(conflict.title.toLowerCase()).toMatch(/someone else|somebody else/);
    expect(conflict.title.toLowerCase()).not.toMatch(/connect|reach|network|offline/);
  });

  it('names the link for an unreachable engine', () => {
    expect(unreachable.title.toLowerCase()).toMatch(/reach|connect/);
    expect(unreachable.title.toLowerCase()).not.toMatch(/someone else|somebody else/);
  });

  it('gives them different words entirely', () => {
    expect(conflict.title).not.toBe(unreachable.title);
    expect(conflict.detail).not.toBe(unreachable.detail);
  });
});

describe('what a failed save promises about the work', () => {
  it('says the changes are still there, whichever way it failed', () => {
    // The developer's first question after any failed save. Leaving it unanswered is what makes
    // somebody close the window and lose the work (FR-011, FR-013).
    for (const label of [conflict, unreachable]) {
      expect(label.detail?.toLowerCase()).toContain('still here');
    }
  });
});

describe('the escape offered', () => {
  it('offers to take the host version only for a conflict', () => {
    expect(conflict.offersReload).toBe(true);
    expect(unreachable.offersReload).toBe(false);
    expect(describeOutcome({ kind: 'written', sha256: 'a' }).offersReload).toBe(false);
    expect(describeOutcome({ kind: 'refused', message: 'no' }).offersReload).toBe(false);
  });

  it('never offers to overwrite the host', () => {
    // FR-012b. Re-reading the current hash and writing over it destroys a colleague's work
    // silently, which §11 names as the failure this product cannot afford. Asserted on every
    // outcome's words, because the one that adds it later will look reasonable in review.
    const outcomes: WriteOutcome[] = [
      { kind: 'written', sha256: 'a' },
      { kind: 'conflict' },
      { kind: 'unreachable' },
      { kind: 'refused', message: 'no' },
    ];
    for (const o of outcomes) {
      const label = describeOutcome(o);
      const words = `${label.title} ${label.detail ?? ''}`.toLowerCase();
      expect(words).not.toMatch(/overwrite|force|anyway|replace theirs/);
    }
  });
});

describe('a refusal', () => {
  it("carries the host's own reason rather than paraphrasing it", () => {
    const label = describeOutcome({ kind: 'refused', message: 'permission denied' });
    expect(label.detail).toBe('permission denied');
  });
});
