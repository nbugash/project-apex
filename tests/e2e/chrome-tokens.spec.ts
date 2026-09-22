// T023 — US1. No chrome dimension resolves to a literal rather than a token (SC-002).
import { waitForShell } from './helpers';

/** Every dimension the chrome renders with must trace back to a custom property. Checking
 *  the computed value would prove nothing — a hard-coded 46px computes to exactly what the
 *  token does. So this reads the authored rules out of the stylesheets instead. */
async function authoredDeclarations(selectors: string[]): Promise<string[]> {
  return browser.execute((wanted: string[]) => {
    const out: string[] = [];
    for (const sheet of Array.from(document.styleSheets)) {
      let rules: CSSRuleList;
      try {
        rules = sheet.cssRules;
      } catch {
        continue; // cross-origin sheet; the design system is same-origin
      }
      for (const rule of Array.from(rules)) {
        const style = (rule as CSSStyleRule).style;
        const selector = (rule as CSSStyleRule).selectorText;
        if (!style || !selector) continue;
        if (!wanted.some((w) => selector.includes(w))) continue;
        for (const prop of Array.from(style)) {
          out.push(`${selector} { ${prop}: ${style.getPropertyValue(prop).trim()} }`);
        }
      }
    }
    return out;
  }, selectors);
}

// Properties whose value is a dimension. A raw px in any of these is the drift this checks
// for. Border widths and the focus ring are excluded for the same reason the adherence
// lint excludes them: the design system carries no 1px or 2px token, so they cannot come
// from one.
const DIMENSION_PROPERTIES =
  /^(inline-size|block-size|width|height|min-inline-size|max-inline-size|font-size|gap|padding|letter-spacing|margin)/;

const EXEMPT = /^(outline|border-width|border-radius: 1px|.*: 0(px)?$)/;

describe('chrome dimensions come from tokens', () => {
  before(async () => {
    await waitForShell();
  });

  it('declares no raw pixel dimension on any chrome surface', async () => {
    const declarations = await authoredDeclarations(['.chrome', '.rail', '.tool-window']);
    expect(declarations.length).toBeGreaterThan(0);

    const offenders = declarations.filter((d) => {
      const [, body] = d.split('{');
      if (!body) return false;
      const [prop, value] = body
        .replace('}', '')
        .split(':')
        .map((s) => s.trim());
      if (!prop || !value) return false;
      if (!DIMENSION_PROPERTIES.test(prop)) return false;
      if (EXEMPT.test(`${prop}: ${value}`)) return false;
      // 1px and 2px are the untokenised hairline and focus-ring values.
      const literals = [...value.matchAll(/(\d+(?:\.\d+)?)px/g)].map((m) => Number(m[1]));
      return literals.some((n) => n !== 1 && n !== 2);
    });

    expect(offenders).toEqual([]);
  });

  it('resolves every layout token the chrome relies on', async () => {
    // A token that is referenced but never defined resolves to nothing, and the element
    // silently renders unstyled. This catches a rename on either side of the boundary.
    const unresolved = await browser.execute(() => {
      const names = [
        '--vk-chrome-height',
        '--vk-chrome-gap',
        '--vk-chrome-pad',
        '--vk-rail-width',
        '--vk-rail-button',
        '--vk-rail-icon',
        '--vk-rail-mark-height',
        '--vk-tool',
        '--vk-tool-header-height',
        '--vk-tool-label-size',
      ];
      const root = getComputedStyle(document.documentElement);
      return names.filter((n) => root.getPropertyValue(n).trim() === '');
    });
    expect(unresolved).toEqual([]);
  });
});
