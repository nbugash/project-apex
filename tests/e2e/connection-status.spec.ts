// T048 — US3. Connection transitions appear without user action (FR-011).
import { waitForShell, relaunch, resetSession } from './helpers';

describe('connection state reporting', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('shows a connection state at launch without being asked', async () => {
    const status = await $('[role="status"]');
    await status.waitForExist({ timeout: 10_000 });
    expect(await status.getText()).not.toBe('');
  });

  it('reflects a transition within five seconds and with no user action', async () => {
    const before = await $('.connection').getText();

    // Drive the stub through the debug-only command. Not a window global: the hook is
    // compiled out of release builds, so it cannot become a production surface.
    await browser.execute(async () => {
      // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
      await window.__TAURI_INTERNALS__.invoke('stub_set_connection', { state: 'connected' });
    });

    await browser.waitUntil(async () => (await $('.connection').getText()) !== before, {
      timeout: 5000,
      timeoutMsg: 'status did not update within the 5s FR-011 budget',
    });
  });
});
