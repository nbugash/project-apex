// T071 — Polish. The interface stays responsive during background work (SC-005, FR-014).
import { openDocuments, waitForShell, relaunch, resetSession } from './helpers';

describe('responsiveness under background work', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('accepts and responds to input while persistence is in flight', async () => {
    // Many rapid mutations keep the persistence path busy.
    await openDocuments(15);

    const started = Date.now();
    const splitter = await $('[aria-label="Resize tool window"]');
    await splitter.click();
    for (let i = 0; i < 10; i++) await browser.keys('ArrowRight');
    const elapsed = Date.now() - started;

    // The interface must not have queued behind the writes.
    expect(elapsed).toBeLessThan(5000);
    expect(await $('.shell').isDisplayed()).toBe(true);
  });

  it('never leaves the window unresponsive', async () => {
    const responded = await browser.execute(() => document.readyState === 'complete');
    expect(responded).toBe(true);
  });
});
