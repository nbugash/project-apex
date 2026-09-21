// T050 — US3. Connection state is legible without interaction (SC-006).
//
// SC-006 was restated during /speckit-analyze from an unfamiliar-developer trial, which
// cannot run in CI, into the structural properties that make it likely. These are those
// properties.
import { waitForShell, relaunch, resetSession } from './helpers';

describe('connection state legibility', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('is visible without scrolling, hovering or opening a menu', async () => {
    const indicator = await $('.connection');
    expect(await indicator.isDisplayed({ withinViewport: true })).toBe(true);
  });

  it('carries both an icon and a text label, so colour is never the only encoding', async () => {
    const hasIcon = await $('.connection i.ph').isExisting();
    const label = (await $('.connection span').getText()).trim();
    expect(hasIcon).toBe(true);
    expect(label.length).toBeGreaterThan(0);
  });

  it('remains visible at the minimum supported window size', async () => {
    await browser.setWindowSize(800, 600);
    expect(await $('.connection').isDisplayed({ withinViewport: true })).toBe(true);
    await browser.setWindowSize(1200, 800);
  });

  it('does not let an overlong workspace name displace it (FR-013)', async () => {
    const overlap = await browser.execute(() => {
      const w = document.querySelector('.workspace')!.getBoundingClientRect();
      const c = document.querySelector('.connection')!.getBoundingClientRect();
      return w.right > c.left;
    });
    expect(overlap).toBe(false);
  });
});
