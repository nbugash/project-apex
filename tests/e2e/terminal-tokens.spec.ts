// T126 — SC-016's rendered half: the panel paints the tokens, and only the tokens.
//
// The source half (`tests/unit/terminal-palette.test.ts`) proves the mapping names three tokens
// and defers eleven. This proves the running application does what that mapping says, which is a
// different claim: a correct table read by nothing, or read and then overridden, looks identical
// from the source.
//
// Asserted on the terminal's own theme rather than on computed style, because the library's
// palette lives in a dependency `lint:ds` cannot see, and because a colour the library supplies
// never appears as a CSS value anywhere.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { waitForShell } from './helpers';

/// The token values `ds-sync` generated, read from the file it writes.
///
/// Read, never restated: a literal here would be a second copy of a value the prototype owns,
/// which is the duplication `ds-sync` exists to prevent.
function token(name: string): string {
  const css = readFileSync(join(process.cwd(), 'client/ui/lib/ds/layout-tokens.css'), 'utf8');
  const found = new RegExp(`${name}:\\s*([^;]+);`).exec(css);
  if (!found?.[1]) throw new Error(`${name} is not in layout-tokens.css; run npm run ds:sync`);
  return found[1].trim();
}

/// A design-system colour, from the stylesheet the prototype ships.
function systemColour(name: string): string {
  const css = readFileSync(join(process.cwd(), 'client/ui/lib/ds/system/styles.css'), 'utf8');
  const found = new RegExp(`${name}:\\s*([^;]+);`).exec(css);
  if (!found?.[1]) throw new Error(`${name} is not in the design system`);
  // A design-system value may carry a trailing comment on the same line; the value is what
  // precedes it.
  return (found[1].trim().split(';')[0] ?? found[1]).trim();
}

async function theme(): Promise<Record<string, string>> {
  await browser.execute((eventName: string) => {
    window.dispatchEvent(
      new CustomEvent(eventName, { detail: { taskId: 'tokens', data: btoa('ready\r\n') } }),
    );
  }, 'apex:test:task-output');
  await browser.waitUntil(
    async () =>
      (await browser.execute(
        () =>
          (window as unknown as { __apexTerminal?: { options?: { theme?: unknown } } })
            .__apexTerminal?.options?.theme !== undefined,
      )) === true,
    { timeout: 15_000, timeoutMsg: 'the panel never acquired a theme' },
  );
  return browser.execute(() => {
    const term = (window as unknown as { __apexTerminal?: unknown }).__apexTerminal as
      { options: { theme?: Record<string, string> } } | undefined;
    return term?.options.theme ?? {};
  });
}

describe('the panel paints the design system (SC-016)', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  it('paints the three hues with the values ds-sync extracted', async () => {
    const painted = await theme();
    expect(painted.red?.toLowerCase()).toBe(token('--vk-term-ansi-red').toLowerCase());
    expect(painted.green?.toLowerCase()).toBe(token('--vk-term-ansi-green').toLowerCase());
    expect(painted.yellow?.toLowerCase()).toBe(token('--vk-term-ansi-yellow').toLowerCase());
  });

  it('takes its background, foreground and cursor from the window', async () => {
    // What keeps the panel from looking like an embedded island.
    const painted = await theme();
    expect(painted.background?.toLowerCase()).toBe(systemColour('--color-bg').toLowerCase());
    expect(painted.foreground?.toLowerCase()).toBe(systemColour('--color-text').toLowerCase());
    expect(painted.cursor?.toLowerCase()).toBe(systemColour('--color-accent').toLowerCase());
  });

  it('leaves the eleven deferred slots to the library', async () => {
    // **Asserted as absent rather than left unasserted**, which is the difference between
    // deferring a decision and forgetting one. A hand-typed hex that happened to look right
    // would pass an assertion that merely checked the three hues.
    const painted = await theme();
    for (const slot of [
      'blue',
      'magenta',
      'cyan',
      'brightBlack',
      'brightRed',
      'brightGreen',
      'brightYellow',
      'brightBlue',
      'brightMagenta',
      'brightCyan',
      'brightWhite',
    ]) {
      expect(`${slot}=${painted[slot] ?? 'unset'}`).toBe(`${slot}=unset`);
    }
  });

  it('measures its cell grid in the font it draws in', async () => {
    // xterm sizes the grid from its **own** `fontFamily` and `fontSize` options, which default
    // to Courier at 15px. Left unset -- as they were -- every cell is laid out to one font's
    // metrics and painted in another's. The visible result is not a missing character but a
    // terminal that does not line up, and a `cols` count the shell is then told, so a prompt
    // that right-aligns anything lands in the wrong place.
    //
    // Asserted as an equality between the two rather than against a value, because the point is
    // that they agree: the stylesheet decides the type and xterm has to follow it, whatever it
    // says.
    const measured = await browser.execute(() => {
      const term = (
        window as unknown as { __apexTerminal?: { options: { fontFamily?: string; fontSize?: number } } }
      ).__apexTerminal;
      const el = document.querySelector('.xterm') as HTMLElement | null;
      if (!term || !el) return null;
      const css = getComputedStyle(el);
      return {
        optionFamily: term.options.fontFamily ?? '',
        optionSize: term.options.fontSize ?? 0,
        cssFamily: css.fontFamily,
        cssSize: Number.parseFloat(css.fontSize),
      };
    });
    expect(measured).not.toBeNull();
    expect(measured!.optionFamily).toBe(measured!.cssFamily);
    expect(measured!.optionSize).toBe(measured!.cssSize);
    // And the family it draws in begins with the one the prototype chose, so everything the
    // design covers is unaffected by the fallbacks appended for everything it does not.
    //
    // Quotes are normalised on both sides: the prototype writes `'JetBrains Mono'` and the
    // browser reports `"JetBrains Mono"`, which is the same family and a different string.
    const unquote = (s: string) => s.replace(/['"]/g, '').trim();
    const first = unquote(measured!.cssFamily.split(',')[0] ?? '');
    expect(first).toBe(unquote(token('--vk-mono-primary')));
  });

  it('has the symbols font the private use area needs', async () => {
    // A shell prompt is built from Powerline separators and Nerd Font icons in the private use
    // area. No text font carries them, so without this face they are blank boxes -- the defect
    // that was reported, and one that a screenshot shows and no assertion about bytes can: the
    // codepoints arrive intact either way.
    //
    // Checked through the font system rather than by measuring a glyph, because a missing glyph
    // and a present one both occupy a cell and the difference is what is painted inside it.
    // `load` rather than `check`: a face that is declared but not yet fetched reports false,
    // and what is being guarded is that the file ships, is wired up, and parses -- all three of
    // which `load` resolving proves and a declaration alone does not.
    const loaded = await browser.execute(async () => {
      // The size is read from the terminal rather than written here. Any size would answer the
      // availability question, but a literal one is this test asserting a value the stylesheet
      // owns -- the same rule the components follow.
      const el = document.querySelector('.xterm') as HTMLElement | null;
      const size = el ? getComputedStyle(el).fontSize : '';
      const spec = `${size} "JetBrains Mono Nerd Symbols"`;
      try {
        const faces = await document.fonts.load(spec, '\ue0b0');
        return { count: faces.length, usable: document.fonts.check(spec) };
      } catch (e) {
        return { count: -1, usable: false, error: String(e) };
      }
    });
    expect(loaded.count).toBeGreaterThan(0);
    expect(loaded.usable).toBe(true);

    // And it is metrically the same cell as the primary font. CSS fallback uses the fallback's
    // own advance, so a symbol font whose advance differs overruns the character beside it --
    // which showed up as `on git` rendering as `on gi`, at every size, because it is a property
    // of the font rather than of the scaling.
    const widths = await browser.execute(() => {
      const measure = (family: string) => {
        const c = document.createElement('canvas').getContext('2d');
        if (!c) return 0;
        c.font = `100px ${family}`;
        return c.measureText('M').width;
      };
      return {
        primary: measure('"JetBrains Mono"'),
        symbols: measure('"JetBrains Mono Nerd Symbols", "JetBrains Mono"'),
      };
    });
    expect(widths.primary).toBeGreaterThan(0);
    expect(widths.symbols).toBeCloseTo(widths.primary, 1);
  });

  it('paints no colour the design system did not give it', async () => {
    // The whole of SC-016 in one assertion: every value in the theme must be one the design
    // system holds. A slot with a value from nowhere is a colour somebody invented.
    const painted = await theme();
    const allowed = new Set(
      [
        token('--vk-term-ansi-red'),
        token('--vk-term-ansi-green'),
        token('--vk-term-ansi-yellow'),
        systemColour('--color-bg'),
        systemColour('--color-text'),
        systemColour('--color-accent'),
      ].map((v) => v.toLowerCase()),
    );
    for (const [slot, value] of Object.entries(painted)) {
      expect(`${slot}:${value.toLowerCase()}`).toBe(
        `${slot}:${allowed.has(value.toLowerCase()) ? value.toLowerCase() : 'NOT-FROM-THE-DESIGN-SYSTEM'}`,
      );
    }
  });
});
