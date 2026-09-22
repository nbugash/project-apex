// T029 — US2. Destination switching and active-state movement (FR-004, FR-005).
//
// Coverage limitation, recorded rather than worked around: exactly one destination is
// available in this release, because every other one's tool window belongs to a later
// feature (FR-008). Switching the active destination *between* two available ones therefore
// cannot be exercised end to end yet. The core's unit tests cover that transition directly;
// what this file asserts is everything the rendered rail can actually be held to today —
// that the available one is active, and that the unavailable ones are inert rather than
// silently broken.
import { waitForShell, resetSession, relaunch } from './helpers';

describe('rail navigation', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('marks exactly one destination active on open', async () => {
    const selected = await $$('nav.rail [role="tab"][aria-selected="true"]');
    expect(selected.length).toBe(1);
  });

  it('marks every destination without a tool window as unavailable', async () => {
    const all = await $$('nav.rail [role="tab"]');
    const unavailable = await $$('nav.rail [role="tab"][aria-disabled="true"]');
    expect(all.length).toBe(6);
    expect(unavailable.length).toBe(5);
  });

  it('does nothing when an unavailable destination is clicked', async () => {
    const before = await browser.execute(
      () =>
        document
          .querySelector('nav.rail [role="tab"][aria-selected="true"]')
          ?.getAttribute('title') ?? null,
    );

    const unavailable = await $$('nav.rail [role="tab"][aria-disabled="true"]');
    await unavailable[0]!.click();

    // The panel must still be open on the same destination. A click that collapsed the
    // panel, or moved the active mark onto a destination with nothing behind it, would
    // both read to a user as the application breaking.
    const after = await browser.execute(
      () =>
        document
          .querySelector('nav.rail [role="tab"][aria-selected="true"]')
          ?.getAttribute('title') ?? null,
    );
    expect(after).toBe(before);
    expect(await $('aside.tool-window').isExisting()).toBe(true);
  });

  it('names the active destination in the tool window header', async () => {
    const railLabel = await browser.execute(
      () =>
        document
          .querySelector('nav.rail [role="tab"][aria-selected="true"]')
          ?.getAttribute('title') ?? null,
    );
    const headerLabel = await browser.execute(
      () => document.querySelector('aside.tool-window .label')?.textContent?.trim() ?? null,
    );
    expect(headerLabel?.toLowerCase()).toBe(railLabel?.toLowerCase());
  });
});
