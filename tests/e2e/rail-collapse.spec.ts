// T030 — US2. Selecting the active destination collapses, and restores at the previous
// width (FR-006).
import { waitForShell, resetSession, relaunch, waitForPersisted } from './helpers';

const activeButton = () => $('nav.rail [role="tab"][aria-selected="true"]');

describe('tool window collapse', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('collapses when the active destination is selected again', async () => {
    expect(await $('aside.tool-window').isExisting()).toBe(true);
    await activeButton().click();

    await browser.waitUntil(async () => !(await $('aside.tool-window').isExisting()), {
      timeout: 5000,
      timeoutMsg: 'the tool window never collapsed',
    });
  });

  it('keeps the rail usable while collapsed', async () => {
    // The only way back is through the rail, so a collapse that hid it too would be a trap.
    expect(await $('nav.rail').isExisting()).toBe(true);
    expect((await $$('nav.rail [role="tab"]')).length).toBe(6);
  });

  it('restores at the width it had before collapsing, not a default', async () => {
    // Widen first so "restored at its previous width" is distinguishable from "restored at
    // the default width" — with the panel at its default, both would pass.
    const splitter = await $('[aria-label="Resize tool window"]');
    expect(await splitter.isExisting()).toBe(false); // collapsed: nothing to drag

    // Re-open, widen, collapse, re-open.
    await $('nav.rail [role="tab"]:not([aria-disabled="true"])').click();
    await $('aside.tool-window').waitForExist({ timeout: 5000 });

    const reopened = await $('[aria-label="Resize tool window"]');
    await reopened.click();
    for (let i = 0; i < 5; i++) await browser.keys('ArrowRight');

    const widened = (await $('aside.tool-window').getSize()).width;
    await waitForPersisted((s) => (s.tool_window as { width: number }).width === widened, 'width');

    await activeButton().click();
    await browser.waitUntil(async () => !(await $('aside.tool-window').isExisting()), {
      timeout: 5000,
      timeoutMsg: 'the tool window never collapsed',
    });

    await activeButton().click();
    await $('aside.tool-window').waitForExist({ timeout: 5000 });
    expect((await $('aside.tool-window').getSize()).width).toBe(widened);
  });
});
