// T025 — US1. Every git state maps to a design-system token, and all five stay apart in
// greyscale (FR-014, FR-015, SC-008).
//
// **Compared by luminance, not by hue**, following `rail-greyscale.spec.ts`. A design that
// carried the whole distinction in colour would pass a test comparing hues perfectly, and would
// be unreadable for the roughly one developer in twelve who cannot separate those hues.
import { describe, it, expect } from 'vitest';
import { markerFor, allMarkers, type GitState } from '../../client/ui/lib/git/marker';
import { readFileSync } from 'node:fs';

const STATES: GitState[] = ['modified', 'untracked', 'staged', 'deleted', 'conflict'];

const SYSTEM = readFileSync('client/ui/lib/ds/system/styles.css', 'utf8');

/** The hex a token resolves to in the design system, so the test reads what ships. */
function valueOf(token: string): string {
  const m = new RegExp(`${token}\\s*:\\s*(#[0-9a-fA-F]{3,8})`).exec(SYSTEM);
  if (!m) throw new Error(`${token} is not a design-system token`);
  return m[1]!;
}

/** Relative luminance, which is what survives a greyscale rendering. */
function luminance(hex: string): number {
  const h = hex.slice(1);
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

describe('git markers', () => {
  it('maps every state the protocol carries', () => {
    for (const state of STATES) {
      const marker = markerFor(state);
      expect(marker, `${state} has no marker`).not.toBeNull();
      expect(marker!.glyph.length).toBe(1);
      expect(marker!.label.length).toBeGreaterThan(0);
    }
    expect(allMarkers().length).toBe(STATES.length);
  });

  it('names only tokens the design system defines', () => {
    // The failure this catches is the one F006's first new surface hit: an invented token name
    // renders as nothing at all, which looks like a missing marker rather than a typo.
    for (const [, marker] of allMarkers()) {
      expect(() => valueOf(marker.token), `${marker.token} is invented`).not.toThrow();
    }
  });

  it('gives every state a distinct glyph, so shape alone separates them', () => {
    const glyphs = allMarkers().map(([, m]) => m.glyph);
    expect(new Set(glyphs).size).toBe(glyphs.length);
  });

  it('separates all five by luminance, so colour is not the only channel', () => {
    const readings = allMarkers()
      .map(([state, m]) => ({ state, lum: luminance(valueOf(m.token)) }))
      .sort((a, b) => a.lum - b.lum);

    for (let i = 1; i < readings.length; i += 1) {
      const gap = readings[i]!.lum - readings[i - 1]!.lum;
      expect(
        gap,
        `${readings[i - 1]!.state} and ${readings[i]!.state} are ${gap.toFixed(1)} apart in greyscale`,
      ).toBeGreaterThan(20);
    }
  });

  it('keeps every marker clear of the window background', () => {
    // A marker that is distinct from the other four and indistinguishable from the ground is
    // still invisible. The four gaps above say nothing about this one.
    const ground = luminance(valueOf('--color-bg'));
    for (const [state, m] of allMarkers()) {
      expect(luminance(valueOf(m.token)) - ground, `${state} is lost in the background`).toBeGreaterThan(60);
    }
  });

  it('shows nothing for a file with no git state', () => {
    // FR-016: a file git says nothing about keeps exactly the row it has today.
    expect(markerFor(undefined)).toBeNull();
    expect(markerFor(null)).toBeNull();
    expect(markerFor('')).toBeNull();
  });

  it('shows nothing for a state this build does not know', () => {
    // Never a default. Marking a file with a state git never reported is worse than not
    // marking it, because the developer has no way to find out it was invented.
    expect(markerFor('renamed')).toBeNull();
    expect(markerFor('MODIFIED')).toBeNull();
  });
});
