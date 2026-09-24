import { describe, expect, it } from 'vitest';
import {
  CONTENT_PRESENTATION,
  MAINTENANCE_PRESENTATION,
  presentContent,
  presentMaintenance,
} from '../../client/ui/lib/statusbar/presentation';

describe('content presentation', () => {
  // FR-039, SC-016. The greyscale gate renders these; this asserts the property the gate
  // depends on, at the level where it can be checked exhaustively.
  it('gives every state an icon and a label, so none is carried by colour alone', () => {
    const states = Object.keys(CONTENT_PRESENTATION);
    expect(states).toHaveLength(6);
    for (const [name, p] of Object.entries(CONTENT_PRESENTATION)) {
      expect(p.icon, `${name} needs a glyph`).toMatch(/^ph-/);
      expect(p.label.length, `${name} needs a label`).toBeGreaterThan(0);
    }
  });

  it('keeps every state distinguishable from every other without colour', () => {
    // Two states sharing both an icon and a label would be identical in a greyscale render,
    // which is exactly what SC-016 forbids.
    const seen = new Set<string>();
    for (const p of Object.values(CONTENT_PRESENTATION)) {
      const key = `${p.icon}|${p.label}`;
      expect(seen.has(key), `duplicate presentation: ${key}`).toBe(false);
      seen.add(key);
    }
  });

  it('distinguishes possibly-stale from could-not-verify', () => {
    // The same bytes with the same hash; they differ only in why nobody could confirm them, and
    // a developer needs to be able to tell which happened.
    expect(presentContent('possiblyStale')).not.toEqual(presentContent('unverified'));
  });

  it('degrades rather than breaking on a state it has not heard of', () => {
    // A newer core reporting a state an older interface does not know must not blank the bar.
    expect(presentContent('something-new').label).toBe('Unknown');
  });
});

describe('maintenance presentation', () => {
  it('renders exactly the three phases the interface shows', () => {
    expect(Object.keys(MAINTENANCE_PRESENTATION)).toEqual(['migrating', 'rebuilding', 'evicting']);
  });

  it('does not render the phases that only order the work', () => {
    for (const phase of ['idle', 'checking', 'ready']) {
      expect(presentMaintenance(phase)).toBeNull();
    }
  });

  it('keeps migrating distinguishable from evicting', () => {
    // Collapsing them into one "maintaining" state would make SC-013a unmeasurable, because it
    // asserts on migration reports specifically.
    expect(presentMaintenance('migrating')).not.toEqual(presentMaintenance('evicting'));
  });

  it('shows the versions, because a label with no movement is indistinguishable from a stall', () => {
    expect(presentMaintenance('migrating', { from: 0, to: 1 })?.label).toContain('v0 to v1');
  });

  it('gives every maintenance state an icon and a label', () => {
    for (const [name, p] of Object.entries(MAINTENANCE_PRESENTATION)) {
      expect(p.icon, `${name} needs a glyph`).toMatch(/^ph-/);
      expect(p.label.length, `${name} needs a label`).toBeGreaterThan(0);
    }
  });
});
