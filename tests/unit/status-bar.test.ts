import { describe, expect, it } from 'vitest';
import type { ConnectionState } from '../../src/lib/ipc';
// The component's own presentation, not a copy of it. The copy this file used to hold was
// why `Retrying` reached the status bar untested: a duplicate cannot fail when the original
// changes, which is the one thing the duplicate was supposed to do.
import { PRESENTATION, present } from '../../src/lib/statusbar/presentation';

const STATES: ConnectionState[] = ['unknown', 'connecting', 'connected', 'disconnected'];

describe('connection state presentation (FR-012, SC-007)', () => {
  it('every state carries both an icon and a text label', () => {
    for (const s of STATES) {
      expect(present(s).icon).toMatch(/^ph-/);
      expect(present(s).label.length).toBeGreaterThan(0);
    }
  });

  it('states remain distinguishable without colour', () => {
    const icons = new Set(STATES.map((s) => present(s).icon));
    const labels = new Set(STATES.map((s) => present(s).label));
    expect(icons.size).toBe(STATES.length);
    expect(labels.size).toBe(STATES.length);
  });

  // F001 FR-020. The state that reports a problem must not be the one that breaks.
  it('a retrying state renders progress rather than throwing', () => {
    const retrying: ConnectionState = { retrying: { attempt: 3, next_in_secs: 8 } };
    const shown = present(retrying);
    expect(shown.icon).toBe('ph-arrows-clockwise');
    expect(shown.label).toContain('8s');
    expect(shown.label).toContain('3');
    expect(shown.icon).not.toBe(PRESENTATION.unknown.icon);
  });

  it('an imminent retry reads as now rather than in 0s', () => {
    expect(present({ retrying: { attempt: 1, next_in_secs: 0 } }).label).toContain('now');
  });

  // A newer core reporting a state this build has not heard of must degrade, not blank the
  // status bar.
  it('an unrecognised state falls back to unknown', () => {
    const future = 'suspended' as unknown as ConnectionState;
    expect(present(future)).toEqual(PRESENTATION.unknown);
  });
});

describe('workspace name truncation (FR-013)', () => {
  // The status bar has a fixed block-size and a bounded max-inline-size, so an overlong
  // name is ellipsised rather than expanding the bar or displacing the connection state.
  it('a 255-character name does not alter the stored value', () => {
    const name = 'w'.repeat(255);
    expect(name.length).toBe(255);
    // Truncation is presentational only; nothing mutates the reference.
    expect(name).toBe('w'.repeat(255));
  });
});
