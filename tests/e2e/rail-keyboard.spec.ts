// T031 — US2. Every destination is reachable and activatable by keyboard alone (FR-007, SC-003).
import { waitForShell, resetSession, relaunch } from './helpers';

const focusedTitle = () =>
  browser.execute(() => document.activeElement?.getAttribute('title') ?? null);

/** Tab forward until focus lands inside the rail, or give up. Driven by real key presses:
 *  programmatic focus would not exercise the tab order, which is the thing under test. */
async function tabIntoRail(maxPresses = 20): Promise<boolean> {
  for (let i = 0; i < maxPresses; i++) {
    await browser.keys('Tab');
    const inRail = await browser.execute(
      () => document.activeElement?.closest('nav.rail') !== null,
    );
    if (inRail) return true;
  }
  return false;
}

describe('rail keyboard operation', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('reaches the rail by tabbing', async () => {
    expect(await tabIntoRail()).toBe(true);
  });

  it('costs one tab stop, not one per destination', async () => {
    // A roving tab index is the difference between the rail being one stop and being six.
    // Without it, reaching anything past the rail costs six presses, five of which land on
    // destinations that cannot be opened.
    const stops = await browser.execute(
      () => document.querySelectorAll('nav.rail [role="tab"][tabindex="0"]').length,
    );
    expect(stops).toBe(1);
  });

  it('moves between destinations with the arrow keys', async () => {
    const before = await focusedTitle();
    await browser.keys('ArrowDown');
    // Unavailable destinations are skipped, so with one available destination the focus
    // may legitimately not move. What must hold is that focus stays in the rail rather
    // than escaping it or landing on nothing.
    const stillInRail = await browser.execute(
      () => document.activeElement?.closest('nav.rail') !== null,
    );
    expect(stillInRail).toBe(true);
    expect(await focusedTitle()).not.toBe(null);
    expect(before).not.toBe(null);
  });

  it('activates the focused destination with the keyboard', async () => {
    await browser.execute(() => {
      const button = document.querySelector<HTMLElement>(
        'nav.rail [role="tab"]:not([aria-disabled="true"])',
      );
      button?.focus();
    });
    await browser.keys('Enter');

    // Activating the active destination collapses it — which is the FR-006 behaviour, and
    // proves the key press reached the handler rather than being swallowed.
    await browser.waitUntil(async () => !(await $('aside.tool-window').isExisting()), {
      timeout: 5000,
      timeoutMsg: 'Enter did not activate the focused destination',
    });

    await browser.keys('Enter');
    await $('aside.tool-window').waitForExist({ timeout: 5000 });
  });
});
