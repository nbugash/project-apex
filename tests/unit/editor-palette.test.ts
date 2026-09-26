/// The editor's colours come from the design system, and keep coming from it.
///
/// Two separate claims, and the second is the one nothing else would catch. The first is that
/// the translation reads tokens rather than writing values. The second is that the tokens this
/// feature maps each syntax role to are still the ones the **prototype** uses for that role — a
/// prototype that changed a colour would otherwise leave the application rendering the old one
/// indefinitely, which is the drift Principle I exists to catch (T003, A-EDITPALETTE).
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { editorTheme, SYNTAX_TOKENS } from '../../client/ui/lib/editor/palette';

/// The tokens as the design system defines them, read from the bundle `ds:sync` copies.
function designSystemColours(): Map<string, string> {
  const css = readFileSync(join(process.cwd(), 'client/ui/lib/ds/system/styles.css'), 'utf8');
  const out = new Map<string, string>();
  for (const m of css.matchAll(/(--color-[a-z0-9-]+)\s*:\s*(#[0-9a-fA-F]{3,8})/g)) {
    if (!out.has(m[1]!)) out.set(m[1]!, m[2]!.toLowerCase());
  }
  return out;
}

/// What the prototype paints each syntax role. Anchored on the code the span wraps, because the
/// content is what makes the colour mean a role — a position in the markup does not.
function prototypeColours(): Record<string, string> {
  const html = readFileSync(join(process.cwd(), 'mockups/Apex IDE (standalone).html'), 'utf8');
  const at = (re: RegExp): string => {
    const m = re.exec(html);
    if (!m) throw new Error(`the prototype no longer contains ${re}`);
    return m[1]!.toLowerCase();
  };
  return {
    keyword: at(/color:(#[0-9a-fA-F]{6})\\?">public/),
    type: at(/color:(#[0-9a-fA-F]{6})\\?">Receipt/),
    identifier: at(/color:(#[0-9a-fA-F]{6})\\?">confirm/),
    delimiter: at(/color:(#[0-9a-fA-F]{6})\\?">\(/),
  };
}

describe('the editor palette', () => {
  it('maps every syntax role to a token the design system defines', () => {
    const ds = designSystemColours();
    for (const [role, token] of SYNTAX_TOKENS) {
      expect(ds.has(token), `${role} maps to ${token}, which the design system does not define`).toBe(true);
    }
  });

  it('gives each role the colour the prototype paints it', () => {
    // The check that keeps the mapping honest. A colour changed in the prototype and not here
    // would leave the editor rendering a value nobody approved, and every other test would pass.
    const ds = designSystemColours();
    const proto = prototypeColours();
    const mapped = new Map(SYNTAX_TOKENS.map(([role, token]) => [role, ds.get(token)]));
    for (const [role, expected] of Object.entries(proto)) {
      expect(mapped.get(role), `role ${role}`).toBe(expected);
    }
  });

  it('reads values from the element rather than writing them', () => {
    // The stub's values are the design system's own, read from disk rather than typed here.
    // Typing them would put colour literals in a test that exists to prove the application
    // contains none -- and `lint:ds` catches exactly that, which is how this was noticed.
    const ds = designSystemColours();
    const values: Record<string, string> = {
      '--color-bg': ds.get('--color-bg')!,
      '--color-text': ds.get('--color-text')!,
      '--color-accent': ds.get('--color-accent')!,
      '--color-accent-400': ds.get('--color-accent-400')!,
    };
    const original = globalThis.getComputedStyle;
    globalThis.getComputedStyle = (() => ({
      getPropertyValue: (t: string) => values[t] ?? '',
    })) as unknown as typeof globalThis.getComputedStyle;
    try {
      const theme = editorTheme({} as HTMLElement);
      expect(theme.colors['editor.background']).toBe(ds.get('--color-bg'));
      expect(theme.rules.find((r) => r.token === 'keyword')?.foreground).toBe(
        ds.get('--color-accent-400')!.replace(/^#/, ''),
      );
    } finally {
      globalThis.getComputedStyle = original;
    }
  });

  it('omits a role whose token the stylesheet does not define', () => {
    // Omitted rather than defaulted: Monaco then shows its own colour, which is visibly wrong
    // and gets fixed. A silent default is invisibly wrong and stays.
    const original = globalThis.getComputedStyle;
    globalThis.getComputedStyle = (() => ({ getPropertyValue: () => '' })) as unknown as typeof globalThis.getComputedStyle;
    try {
      const theme = editorTheme({} as HTMLElement);
      expect(theme.rules).toEqual([]);
      expect(Object.keys(theme.colors)).toEqual([]);
    } finally {
      globalThis.getComputedStyle = original;
    }
  });
});
