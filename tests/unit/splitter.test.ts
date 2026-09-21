import { describe, expect, it } from 'vitest';
import { clampExtent, deltaFor, MIN_REGION_EXTENT } from '../../src/lib/shell/extent';

describe('region extent clamping (FR-004)', () => {
  it('holds at the minimum rather than collapsing', () => {
    expect(clampExtent(10)).toBe(MIN_REGION_EXTENT);
    expect(clampExtent(0)).toBe(MIN_REGION_EXTENT);
    expect(clampExtent(-500)).toBe(MIN_REGION_EXTENT);
  });

  it('passes values at or above the minimum through untouched', () => {
    expect(clampExtent(MIN_REGION_EXTENT)).toBe(MIN_REGION_EXTENT);
    expect(clampExtent(400)).toBe(400);
  });

  it('respects an upper bound when one is given', () => {
    expect(clampExtent(9999, MIN_REGION_EXTENT, 600)).toBe(600);
  });

  it('a region cannot be dragged into an unrecoverable state', () => {
    // The user drags hard to the left; the region must remain grabbable.
    let extent = 300;
    for (let i = 0; i < 50; i++) extent = clampExtent(extent - 40);
    expect(extent).toBeGreaterThanOrEqual(MIN_REGION_EXTENT);
  });
});

describe('splitter axis handling', () => {
  it('a vertical splitter follows horizontal movement', () => {
    expect(deltaFor('vertical', 12, 0)).toBe(12);
  });

  it('the output region grows as the pointer moves up', () => {
    expect(deltaFor('horizontal', 0, -12)).toBe(12);
  });
});
