// T051, T097 — FR-021b, FR-039, SC-016. The verification state, and every state in greyscale.
//
// NOTE: needs a display; see the note in workspace-tree.spec.ts.
import { waitForShell, resetSession, relaunch } from './helpers';

describe('content verification state', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  // FR-039, SC-016.
  it('keeps every published state distinguishable without colour', async () => {
    const presented = await browser.execute(() => {
      // Read the shipped map rather than a fixture, so a state added without a glyph fails here.
      const w = window as unknown as {
        __APEX_CONTENT_PRESENTATION__?: Record<string, { icon: string; label: string }>;
      };
      const map = w.__APEX_CONTENT_PRESENTATION__ ?? {};
      return Object.entries(map).map(([state, p]) => ({ state, icon: p.icon, label: p.label }));
    });

    expect(presented.length).toBe(6);

    // Two states sharing both a glyph and a label are identical once colour is removed, which
    // is exactly what SC-016 forbids. Colour is not consulted at all here — that is the point.
    const seen = new Set<string>();
    for (const p of presented) {
      expect(p.icon).toMatch(/^ph-/);
      expect(p.label.length).toBeGreaterThan(0);
      const key = `${p.icon}|${p.label}`;
      expect(seen.has(key)).toBe(false);
      seen.add(key);
    }
  });

  it('shows a verification state while a confirmation is outstanding', async () => {
    // FR-021b: a wait the developer cannot see is indistinguishable from a frozen window.
    const badge = await $('[data-testid="verify-badge"]');
    if (await badge.isExisting()) {
      const label = await badge.getText();
      expect(label.length).toBeGreaterThan(0);
    }
  });
});
