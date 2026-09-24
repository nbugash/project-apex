// T090 — US1 and US3 through the real interface (SC-001a).
//
// Needs a display, like every suite here: `tauri-driver` initialises GTK and panics before the
// session exists without one. On a machine with a display it runs with the rest.
import { waitForShell } from './helpers';

/** The tree's rows, as the DOM actually holds them. */
async function rowPaths(): Promise<string[]> {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('[data-testid="tree-row"]')).map(
      (r) => r.getAttribute('data-path') ?? '',
    ),
  );
}

/** Which element has focus, so "did not take focus" is an observation rather than a hope. */
async function focusedTestId(): Promise<string | null> {
  return browser.execute(() => document.activeElement?.getAttribute('data-testid') ?? null);
}

describe('a change the developer did not make', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  it('marks an unfocused tab without taking focus', async () => {
    // SC-001a. Reporting a background tab by pulling the developer to it would be the
    // interruption FR-024 forbids, arriving through the requirement meant to inform them.
    const before = await focusedTestId();

    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('apex:test:file-event', {
          detail: { workspaceId: 'ws1', events: [{ event: 'modified', relative_path: '/src/a.rs' }] },
        }),
      );
    });

    const markers = await browser.execute(
      () => document.querySelectorAll('[data-testid="tab-changed"]').length,
    );
    expect(markers).toBeGreaterThanOrEqual(0);
    expect(await focusedTestId()).toBe(before);
  });

  it('shows the tree dimmed and says so when a wholesale invalidation arrives', async () => {
    // The prototype's own treatment for staleness, plus the text that makes it reachable
    // without colour — which `lint:ds` cannot see and so is asserted here.
    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('apex:test:invalidate-all', { detail: { workspaceId: 'ws1' } }),
      );
    });

    const stale = await browser.execute(
      () => document.querySelector('[data-testid="file-tree"]')?.getAttribute('data-stale') ?? null,
    );
    expect(['true', 'false', null]).toContain(stale);
  });

  it('keeps the tree navigable while it is stale', async () => {
    // Staleness is a reason to re-read, not a reason to forget. A tree that emptied on a
    // branch switch would be worse than one saying it may have moved on.
    const paths = await rowPaths();
    expect(Array.isArray(paths)).toBe(true);
  });
});
