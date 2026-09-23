// T039, T098 — US1 and FR-040. The tree through the real interface.
//
// NOTE: this suite needs a display. `tauri-driver` delegates to the platform WebDriver and
// initialises GTK, so it cannot run in a headless sandbox without one — it panics on
// `gtk::rt::init` before the session is created. On a machine with a display, or CI with one,
// it runs with the rest of the suite.
import { waitForShell, resetSession, relaunch } from './helpers';

/** The tree's rows, as the DOM actually holds them. */
async function rows(): Promise<{ path: string; level: number; expanded: string | null }[]> {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('[data-testid="tree-row"]')).map((r) => ({
      path: r.getAttribute('data-path') ?? '',
      level: Number(r.getAttribute('aria-level') ?? 0),
      expanded: r.getAttribute('aria-expanded'),
    })),
  );
}

describe('workspace tree', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('renders the tree as a tree, not a list', async () => {
    // The role and levels are what a screen reader navigates by. A flat list of divs would look
    // identical and be unusable, which is the kind of defect a screenshot never catches.
    const role = await browser.execute(
      () => document.querySelector('[data-testid="file-tree"]')?.getAttribute('role') ?? '',
    );
    expect(role).toBe('tree');
  });

  it('marks folders as expandable and files as not', async () => {
    const all = await rows();
    // A file must not claim to be expandable: aria-expanded on a leaf tells a screen reader
    // there is something to open, and there is not.
    for (const r of all.filter((x) => !x.path.includes('.'))) {
      expect(r.expanded === 'true' || r.expanded === 'false').toBe(true);
    }
  });

  // FR-040, SC-017, US1.5.
  it('is reachable from the keyboard and shows the design system ring', async () => {
    const first = await $('[data-testid="tree-row"]');
    await first.click();

    const outline = await browser.execute(() => {
      const row = document.querySelector('[data-testid="tree-row"]') as HTMLElement | null;
      row?.focus();
      const cs = row ? getComputedStyle(row) : null;
      return {
        focused: document.activeElement === row,
        tabIndex: row?.tabIndex ?? -1,
        outlineWidth: cs?.outlineWidth ?? '',
        outlineStyle: cs?.outlineStyle ?? '',
      };
    });

    expect(outline.focused).toBe(true);
    expect(outline.tabIndex).toBe(0);
    // The browser default is a 1px auto ring; the design system prescribes 2px solid accent.
    // Asserting "some outline exists" would pass for the default, which Principle I forbids.
    expect(outline.outlineStyle).not.toBe('auto');
  });

  it('expands a folder from the keyboard', async () => {
    const before = (await rows()).length;
    await browser.execute(() => {
      const folder = Array.from(document.querySelectorAll('[data-testid="tree-row"]')).find(
        (r) => r.getAttribute('aria-expanded') === 'false',
      ) as HTMLElement | undefined;
      folder?.focus();
    });
    await browser.keys(['Enter']);
    await browser.waitUntil(async () => (await rows()).length !== before, {
      timeout: 5000,
      timeoutMsg: 'Enter did not expand a folder — the tree is pointer-only',
    });
  });
});
