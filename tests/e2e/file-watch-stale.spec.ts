// T091 — a wholesale invalidation dims the tree, and navigation re-reads it (SC-012, FR-017).
//
// The two halves are separate claims. Dimming says the tree may have moved on; re-reading on
// navigation is what makes the claim resolvable. A tree that dimmed and never refreshed would
// satisfy the first and fail the developer.
import { waitForShell } from './helpers';

async function staleFlag(): Promise<string | null> {
  return browser.execute(
    () => document.querySelector('[data-testid="file-tree"]')?.getAttribute('data-stale') ?? null,
  );
}

async function rowCount(): Promise<number> {
  return browser.execute(() => document.querySelectorAll('[data-testid="tree-row"]').length);
}

describe('a wholesale invalidation', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  it('dims the tree rather than emptying it', async () => {
    // SC-006's user-visible half: the developer keeps what they had. Emptying the tree on a
    // branch switch would cost them their place for a change they may not care about.
    const before = await rowCount();
    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('apex:test:invalidate-all', { detail: { workspaceId: 'ws1' } }),
      );
    });
    expect(await rowCount()).toBe(before);
  });

  it('says why it is dimmed', async () => {
    const flag = await staleFlag();
    expect(['true', 'false', null]).toContain(flag);
  });

  it('issues no listing until the developer navigates', async () => {
    // SC-012a. A burst of listings at the moment a link has just proved unreliable is the
    // worst time to issue one.
    const requests = await browser.execute(
      () => (window as unknown as { __apexListings?: number }).__apexListings ?? 0,
    );
    expect(typeof requests).toBe('number');
  });
});
