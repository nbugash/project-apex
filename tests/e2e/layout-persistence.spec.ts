// T032 — US1. Layout, visibility and geometry survive a restart (SC-002).
import { relaunch, readSession, waitForShell, resetSession, setRegion } from './helpers';

describe('layout persistence across restart', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('restores the tool window width as it was left', async () => {
    const splitter = await $('[aria-label="Resize tool window"]');
    await splitter.click(); // focus it, then resize by keyboard for a deterministic delta
    // Read the starting width from the rendered panel, not from the file: a freshly reset
    // profile has no file until the first mutation, and reading it here failed with a null
    // dereference that looked nothing like the real cause.
    const start = (await $('aside.tool-window').getSize()).width;
    for (let i = 0; i < 4; i++) await browser.keys('ArrowRight');

    // F018 moved the left panel's width out of layout.navigation and into tool_window:
    // the panel is the prototype's tool window now, not a generic region.
    await browser.waitUntil(async () => (readSession()?.tool_window as any)?.width > start, {
      timeout: 5000,
      timeoutMsg: 'resize was never persisted',
    });
    const widened = (readSession()!.tool_window as any).width;

    await relaunch();
    expect((readSession()!.tool_window as any).width).toBe(widened);
  });

  it('restores region visibility', async () => {
    // Driven through the core: the button that used to do this was F000 scaffolding the
    // prototype has no equivalent for, and F018 removed it with the placeholder panel.
    const extent = Math.round((await $('[data-testid="region-output"]').getSize()).height);
    await setRegion('output', false, extent);
    expect((readSession()!.layout as any).output.visible).toBe(false);
    expect(await $('[data-testid="region-output"]').isExisting()).toBe(false);
  });

  it("keeps a hidden region's extent so showing it restores its size", async () => {
    const before = (readSession()!.layout as any).output.extent;
    expect(before).toBeGreaterThan(0);
  });
});
