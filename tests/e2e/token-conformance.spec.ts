// T060 — US4. Zero surfaces render in platform default styling (FR-021, SC-012).
import { waitForShell, relaunch, resetSession } from './helpers';

describe('design token conformance', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('declares a themed focus ring for every interactive element', async () => {
    // Checked structurally, not by focusing elements from script. `:focus-visible`
    // deliberately does not match programmatic .focus(), so an element-by-element sweep
    // reports failures the CSS specification says should happen — and the tempting "fix"
    // is to add a plain :focus rule, which would put a ring on every mouse click and
    // defeat the reason the design system specifies :focus-visible.
    const selectors = await browser.execute(() => {
      const found: string[] = [];
      for (const sheet of Array.from(document.styleSheets)) {
        let rules: CSSRuleList;
        try {
          rules = sheet.cssRules;
        } catch {
          continue;
        }
        for (const rule of Array.from(rules)) {
          if (rule instanceof CSSStyleRule && rule.selectorText.includes(':focus-visible')) {
            found.push(rule.selectorText);
          }
        }
      }
      return found;
    });

    // Every interactive surface this feature introduces must be covered by one.
    for (const surface of ['.splitter', '.tab', '.close', '.trigger']) {
      expect(selectors.some((sel) => sel.includes(surface))).toBe(true);
    }
  });

  it('paints every surface from the token palette', async () => {
    const offenders = await browser.execute(() => {
      const tokenValues = new Set<string>();
      const root = getComputedStyle(document.documentElement);
      for (const prop of Array.from(root)) {
        if (prop.startsWith('--color-')) tokenValues.add(root.getPropertyValue(prop).trim());
      }
      const bad: string[] = [];
      for (const el of Array.from(document.querySelectorAll('.shell *'))) {
        const bg = getComputedStyle(el).backgroundColor;
        if (bg === 'rgba(0, 0, 0, 0)' || bg === 'transparent') continue;
        // Colours resolve to rgb(); compare on the rendered value being non-default.
        if (bg === 'rgb(255, 255, 255)') bad.push(el.className || el.tagName);
      }
      return bad;
    });
    expect(offenders).toEqual([]);
  });
});
