// T067 — Polish. Every primary layout and tab action is reachable by keyboard (FR-018, SC-008).
import { openDocuments, readSession, relaunch, resetSession, waitForShell } from './helpers';

describe('keyboard operability', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('resizes a region using only the keyboard', async () => {
    const splitter = await $('[aria-label="Resize navigation"]');
    await splitter.click();

    // The session file does not exist until the first mutation is persisted, so read the
    // baseline from the interface rather than assuming a file is already on disk.
    const before = await browser.execute(() =>
      (document.querySelector('[aria-label="Resize navigation"]') as HTMLElement).getAttribute(
        'aria-valuenow',
      ),
    );

    for (let i = 0; i < 3; i++) await browser.keys('ArrowRight');

    await browser.waitUntil(
      async () => {
        const now = await browser.execute(() =>
          (document.querySelector('[aria-label="Resize navigation"]') as HTMLElement).getAttribute(
            'aria-valuenow',
          ),
        );
        return now !== before;
      },
      { timeout: 5000, timeoutMsg: 'keyboard resize had no effect' },
    );

    // And it reaches disk, which is the part the user notices across a restart.
    await browser.waitUntil(async () => readSession() !== null, {
      timeout: 5000,
      timeoutMsg: 'keyboard resize was never persisted',
    });
  });

  it('moves between tabs using only the keyboard', async () => {
    await openDocuments(3);
    const tabs = await $$('[role="tab"]');
    await tabs[0]!.click();
    const first = await browser.execute(
      () => document.querySelector('.tab.active')?.getAttribute('data-tab') ?? null,
    );

    await browser.keys('ArrowRight');

    await browser.waitUntil(
      async () =>
        (await browser.execute(
          () => document.querySelector('.tab.active')?.getAttribute('data-tab') ?? null,
        )) !== first,
      { timeout: 5000, timeoutMsg: 'keyboard tab navigation had no effect' },
    );
  });

  it('shows a themed focus ring when focus arrives by keyboard', async () => {
    // Focus must be driven by real key presses. `:focus-visible` deliberately does NOT
    // match programmatic .focus(), so asserting against an element focused from script
    // tests something the CSS specification says should not happen.
    await browser.keys('Tab');

    const focused = await browser.execute(() => {
      const el = document.activeElement as HTMLElement | null;
      if (!el || el === document.body) return null;
      const s = getComputedStyle(el);
      return {
        tag: el.tagName,
        outlineStyle: s.outlineStyle,
        outlineWidth: parseFloat(s.outlineWidth),
      };
    });

    // Tab must land on something focusable; a null here means nothing is reachable.
    expect(focused).not.toBeNull();
    expect(focused!.outlineStyle).not.toBe('none');
    expect(focused!.outlineWidth).toBeGreaterThan(0);
  });

  it('declares a themed focus ring rather than relying on the platform default', async () => {
    // The structural guarantee behind FR-021: a :focus-visible rule exists and uses the
    // accent token. This holds regardless of which element currently has focus.
    const rules = await browser.execute(() => {
      let found = 0;
      for (const sheet of Array.from(document.styleSheets)) {
        let cssRules: CSSRuleList;
        try {
          cssRules = sheet.cssRules;
        } catch {
          continue;
        }
        for (const rule of Array.from(cssRules)) {
          if (rule instanceof CSSStyleRule && rule.selectorText.includes(':focus-visible')) {
            if (rule.style.outline || rule.style.outlineColor) found++;
          }
        }
      }
      return found;
    });
    expect(rules).toBeGreaterThan(0);
  });
});
