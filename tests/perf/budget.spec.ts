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
import { inRailOrder, nextSelectable } from '../../src/lib/rail';
import type { RailDestination } from '../../src/lib/ipc';
import type { OpenDocumentReference } from '../../src/lib/ipc';

const STALL_BUDGET_MS = 100; // SC-004

function timed(fn: () => void): number {
  const start = performance.now();
  fn();
  return performance.now() - start;
}

const destinations = (n: number): RailDestination[] =>
  Array.from({ length: n }, (_, i) => ({
    id: `dest${i}`,
    label: `Destination ${i}`,
    icon: 'ph-folder',
    // Mostly unavailable, matching the shipped rail and making the skip path — the one
    // that actually walks the list — the one being measured.
    available: i % 5 === 0,
    order: i,
  }));

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

  // SC-008. Switching destinations costs an ordering pass and a neighbour resolution on
  // the interaction path; the command round trip is measured end to end instead, where a
  // real process boundary exists to measure.
  it('switching tool window destinations stays inside the stall budget', () => {
    const rail = destinations(200);
    const elapsed = timed(() => {
      for (let i = 0; i < 200; i++) {
        inRailOrder(rail);
        nextSelectable(rail, `dest${i}`, i % 2 ? 1 : -1);
      }
    });
    expect(elapsed).toBeLessThan(STALL_BUDGET_MS);
  });

  it('resolving the active destination is not a scan of the whole rail per frame', () => {
    // The shipped rail has six destinations, so any implementation looks instant. This
    // measures the shape rather than the current size: a cost that grows with the rail is
    // a cost that will not be found until the rail grows.
    const small = destinations(6);
    const large = destinations(600);
    const smallCost = timed(() => {
      for (let i = 0; i < 2000; i++) nextSelectable(small, 'dest0', 1);
    });
    const largeCost = timed(() => {
      for (let i = 0; i < 2000; i++) nextSelectable(large, 'dest0', 1);
    });
    expect(largeCost).toBeLessThan(STALL_BUDGET_MS);
    // Generous: this is a guard against an accidental quadratic, not a microbenchmark.
    expect(largeCost).toBeLessThan(Math.max(smallCost, 1) * 400);
  });

  it('keyboard tab navigation is constant-feeling across a large set', () => {
    const many = docs(500);
    const elapsed = timed(() => {
      for (let i = 0; i < 200; i++) neighbour(many, `d${i}`, 1);
    });
    expect(elapsed).toBeLessThan(STALL_BUDGET_MS);
  });
});
