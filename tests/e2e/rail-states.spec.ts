// T034 — US2. A screenshot per rail state, for review against the prototype.
//
// These tests assert little on purpose. Their output is the images: the invisible-window
// defect in F000 passed every assertion in the suite and was found by a human looking at a
// capture. The afterTest hook writes one per test, so each state below becomes a reviewable
// artifact under reports/screenshots/${os}/.
import { waitForShell, resetSession, relaunch } from './helpers';

/** A capture of a blank or unmapped window is worse than no capture: it looks like
 *  evidence. Each state confirms the surface it is documenting is actually on screen. */
async function assertRendered(): Promise<void> {
  const { width, height } = await $('nav.rail').getSize();
  expect(width).toBeGreaterThan(0);
  expect(height).toBeGreaterThan(0);
}

describe('rail states', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('captures the rail with the default destination active', async () => {
    expect((await $$('nav.rail [role="tab"][aria-selected="true"]')).length).toBe(1);
    await assertRendered();
  });

  it('captures the rail with each destination focused in turn', async () => {
    const buttons = await $$('nav.rail [role="tab"]');
    expect(buttons.length).toBe(6);
    for (const button of buttons) {
      await browser.execute((el: HTMLElement) => el.focus(), button as unknown as HTMLElement);
    }
    await assertRendered();
  });

  it('captures the collapsed tool window', async () => {
    await $('nav.rail [role="tab"][aria-selected="true"]').click();
    await browser.waitUntil(async () => !(await $('aside.tool-window').isExisting()), {
      timeout: 5000,
      timeoutMsg: 'the tool window never collapsed',
    });
    await assertRendered();
  });

  it('captures the restored tool window', async () => {
    await $('nav.rail [role="tab"][aria-selected="true"]').click();
    await $('aside.tool-window').waitForExist({ timeout: 5000 });
    await assertRendered();
  });
});
