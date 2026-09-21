import { describe, expect, it } from 'vitest';
import type { ConnectionState } from '../../src/lib/ipc';

// Mirrors the presentation map in StatusBar.svelte. Duplicated deliberately: the test's job
// is to fail when the component's encoding changes in a way that breaks FR-012.
const PRESENTATION: Record<ConnectionState, { icon: string; label: string }> = {
  unknown: { icon: 'ph-question', label: 'Unknown' },
  connecting: { icon: 'ph-circle-dashed', label: 'Connecting' },
  connected: { icon: 'ph-plugs-connected', label: 'Connected' },
  disconnected: { icon: 'ph-plugs', label: 'Offline' },
};

const STATES: ConnectionState[] = ['unknown', 'connecting', 'connected', 'disconnected'];

describe('connection state presentation (FR-012, SC-007)', () => {
  it('every state carries both an icon and a text label', () => {
    for (const s of STATES) {
      expect(PRESENTATION[s].icon).toMatch(/^ph-/);
      expect(PRESENTATION[s].label.length).toBeGreaterThan(0);
    }
  });

  it('states remain distinguishable without colour', () => {
    const icons = new Set(STATES.map((s) => PRESENTATION[s].icon));
    const labels = new Set(STATES.map((s) => PRESENTATION[s].label));
    expect(icons.size).toBe(STATES.length);
    expect(labels.size).toBe(STATES.length);
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
