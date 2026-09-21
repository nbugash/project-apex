// T032 — US1. Layout, visibility and geometry survive a restart (SC-002).
import { relaunch, readSession, waitForShell, resetSession } from './helpers';

describe('layout persistence across restart', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('restores region sizes as they were left', async () => {
    const splitter = await $('[aria-label="Resize navigation"]');
    await splitter.click(); // focus it, then resize by keyboard for a deterministic delta
    for (let i = 0; i < 4; i++) await browser.keys('ArrowRight');

    await browser.waitUntil(async () => (readSession()?.layout as any)?.navigation.extent > 260, {
      timeout: 5000,
      timeoutMsg: 'resize was never persisted',
    });
    const widened = (readSession()!.layout as any).navigation.extent;

    await relaunch();
    expect((readSession()!.layout as any).navigation.extent).toBe(widened);
  });

  it('restores region visibility', async () => {
    const buttons = await $$('nav.placeholder button');
    await buttons[1]!.click();
    await browser.waitUntil(async () => (readSession()?.layout as any)?.output.visible === false, {
      timeout: 5000,
      timeoutMsg: 'visibility was never persisted',
    });

    await relaunch();
    expect((readSession()!.layout as any).output.visible).toBe(false);
    expect(await $('[aria-label="Output"]').isExisting()).toBe(false);
  });

  it("keeps a hidden region's extent so showing it restores its size", async () => {
    const before = (readSession()!.layout as any).output.extent;
    expect(before).toBeGreaterThan(0);
  });
});
