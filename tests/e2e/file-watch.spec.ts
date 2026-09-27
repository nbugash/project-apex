// T090 — US1 and US3 through the real interface (SC-001a).
//
// Needs a display, like every suite here: `tauri-driver` initialises GTK and panics before the
// session exists without one. On a machine with a display it runs with the rest.
import { waitForShell } from './helpers';

/// "Not interrupted" means no modal and no focus steal. A tree row updating in place is not an
/// interruption; a prompt is.
async function dialogsOrPrompts(): Promise<number> {
  return browser.execute(
    () => document.querySelectorAll('[role="dialog"], [role="alertdialog"]').length,
  );
}

/// Does the tree say it may have moved on?
async function staleFlag(): Promise<string | null> {
  return browser.execute(
    () => document.querySelector('[data-testid="file-tree"]')?.getAttribute('data-stale') ?? null,
  );
}

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

    // **Focus, asserted as an identity rather than as a range.** The previous version of this
    // spec asserted `markers >= 0`, which is true of every number a browser can produce and was
    // true throughout the two features in which no file event reached the client at all.
    //
    // What can be claimed without an open tab is the half this test is named for: delivering an
    // event moves focus nowhere. The marker itself belongs to a buffer, and a buffer needs an
    // engine to have loaded a file -- which is `editor-echo.spec.ts`, in the live suite.
    expect(await focusedTestId()).toBe(before);
    expect(await dialogsOrPrompts()).toBe(0);
  });

  it('shows the tree dimmed and says so when a wholesale invalidation arrives', async () => {
    // **The assertion is that the flag changes.** It used to be
    // `expect(['true','false',null]).toContain(stale)` -- every value the attribute can hold,
    // including the one it holds when nothing listened, which is exactly what was happening:
    // `apex:test:invalidate-all` was dispatched into a window with no listener for it, for two
    // whole features.
    expect(await staleFlag()).toBe('false');

    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('apex:test:invalidate-all', { detail: { workspaceId: 'ws1' } }),
      );
    });

    await browser.waitUntil(async () => (await staleFlag()) === 'true', {
      timeout: 10_000,
      timeoutMsg: 'the tree never said it may have moved on',
    });
  });

  it('keeps whatever rows it had when it went stale', async () => {
    // Staleness is a reason to re-read, not a reason to forget: a tree that emptied on a branch
    // switch would be worse than one saying it may have moved on.
    //
    // **This suite runs without an engine**, so the tree here holds no rows and the comparison
    // below is between two empty lists. That is stated rather than dressed up: what it really
    // asserts is that invalidation does not *introduce* rows or throw. The version with rows in
    // it is `git-status.spec.ts`'s count across an update, which needs a real listing and
    // therefore the live suite.
    const before = await rowPaths();
    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('apex:test:invalidate-all', { detail: { workspaceId: 'ws1' } }),
      );
    });
    await browser.waitUntil(async () => (await staleFlag()) === 'true', { timeout: 10_000 });
    expect(await rowPaths()).toEqual(before);
  });
});
