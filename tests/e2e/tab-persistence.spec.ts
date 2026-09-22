// T041 — US2. Tab set, order and focus survive a restart (SC-002).
import { openDocuments, readSession, relaunch, waitForShell, resetSession } from './helpers';

describe('tab persistence across restart', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('restores the same documents in the same order with the same one focused', async () => {
    await openDocuments(3);
    await browser.waitUntil(async () => (readSession()?.documents as any[])?.length === 3, {
      timeout: 5000,
      timeoutMsg: 'documents were never persisted',
    });

    const tabs = await $$('.strip [role="tab"]');
    await tabs[1]!.click();
    const focusedBefore = await browser.waitUntil(
      async () => readSession()?.focused_document_id as string,
      { timeout: 5000, timeoutMsg: 'focus was never persisted' },
    );
    const orderBefore = (readSession()!.documents as any[]).map((d) => d.id);

    await relaunch();

    const after = readSession()!;
    expect((after.documents as any[]).map((d) => d.id)).toEqual(orderBefore);
    expect(after.focused_document_id).toBe(focusedBefore);
    expect(await $$('.strip [role="tab"]')).toHaveLength(3);
  });

  it('re-packs order contiguously after a close', async () => {
    const tabs = await $$('.strip [role="tab"]');
    await tabs[0]!.$('.close').click();
    await browser.waitUntil(async () => (readSession()?.documents as any[])?.length === 2, {
      timeout: 5000,
      timeoutMsg: 'close was never persisted',
    });
    const orders = (readSession()!.documents as any[]).map((d) => d.order);
    expect(orders).toEqual([0, 1]);
  });
});
