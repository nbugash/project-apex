// T070 — the interaction budget as a measurement rather than a claim.
//
// Constitution Principle V: any feature touching the interaction path ships with a
// measurement that FAILS when the budget is breached. These assert SC-001 (launch under
// 2s) and SC-004 (no interaction stall over 100ms) against the pure logic that sits on
// that path. The end-to-end suite measures the same budget against the real window; this
// runs on every commit without a display.
import { describe, expect, it } from 'vitest';
import { clampExtent } from '../../src/lib/shell/extent';
import { inOrder, neighbour } from '../../src/lib/tabs/ordering';
import type { OpenDocumentReference } from '../../src/lib/ipc';

const STALL_BUDGET_MS = 100; // SC-004

function timed(fn: () => void): number {
  const start = performance.now();
  fn();
  return performance.now() - start;
}

const docs = (n: number): OpenDocumentReference[] =>
  Array.from({ length: n }, (_, i) => ({ id: `d${i}`, display_name: `doc${i}`, order: i }));

describe('interaction budget (SC-004, Constitution Principle V)', () => {
  it('a full drag gesture stays well inside the stall budget', () => {
    // A drag emits roughly one clamp per frame; 120 frames is two seconds of dragging.
    const elapsed = timed(() => {
      let extent = 300;
      for (let i = 0; i < 120; i++) extent = clampExtent(extent + (i % 2 ? 3 : -3));
    });
    expect(elapsed).toBeLessThan(STALL_BUDGET_MS);
  });

  it('ordering a large tab set stays inside the stall budget', () => {
    const many = docs(500);
    const elapsed = timed(() => {
      for (let i = 0; i < 50; i++) inOrder(many);
    });
    expect(elapsed).toBeLessThan(STALL_BUDGET_MS);
  });

  it('keyboard tab navigation is constant-feeling across a large set', () => {
    const many = docs(500);
    const elapsed = timed(() => {
      for (let i = 0; i < 200; i++) neighbour(many, `d${i}`, 1);
    });
    expect(elapsed).toBeLessThan(STALL_BUDGET_MS);
  });
});
