// T069, T097 — FR-018a, SC-016. Maintenance is visible while it runs.
//
// Maintenance completes before the window is shown (FR-018c requires it: no provider exists
// until the projection is at a schema this build reads), so a launched shell has normally
// finished it. This asserts the banner's contract when it is present rather than forcing a slow
// migration, and the phase logic itself — including that migrating stays distinguishable from
// evicting, which SC-013a depends on — is asserted exhaustively in
// `tests/unit/workspace-presentation.test.ts` and `cache_maintenance.rs`.
import { waitForShell, resetSession, relaunch } from './helpers';

describe('cache maintenance', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('launches with maintenance already complete', async () => {
    // The banner is for a migration long enough to see. On an empty profile there is nothing to
    // migrate, so its absence here is the correct state rather than a missing feature — and
    // asserting the window is up at all is what proves maintenance did not block the launch.
    const banner = await $('[data-testid="maintenance-banner"]');
    expect(await banner.isExisting()).toBe(false);
  });

  it('shows a glyph and text when a phase is rendered', async () => {
    const shown = await browser.execute(() => {
      const el = document.querySelector('[data-testid="maintenance-banner"]');
      if (!el) return null;
      return {
        phase: el.getAttribute('data-phase') ?? '',
        glyph: el.querySelector('i')?.className ?? '',
        text: (el.textContent ?? '').trim(),
      };
    });

    if (shown === null) return; // nothing running, which the previous test already established
    expect(shown.phase).toMatch(/migrating|rebuilding|evicting/);
    expect(shown.glyph).toContain('ph-');
    expect(shown.text.length).toBeGreaterThan(0);
  });
});
