/// SC-016's source half: every terminal colour is decided in one file, and three of them are ours.
///
/// A-TERMPALETTE: the design system defines three of the sixteen ANSI colours, and the rest come
/// from the terminal library's own palette as a **named** exception. Naming them is the point --
/// an omission and an oversight look identical in code, and the list is the visible marker that a
/// decision is still outstanding.
///
/// Checkable by reading one file, which is what keeping the whole mapping in `palette.ts` buys.
import { describe, expect, it } from 'vitest';
import { ANSI_TOKENS, DEFERRED_ANSI, SURFACE_TOKENS } from '../../client/ui/lib/terminal/palette';

/// The sixteen ANSI slots a terminal renders.
const ANSI_SLOTS = 16;

describe('the terminal palette (SC-016, A-TERMPALETTE)', () => {
  it('maps exactly three ANSI names to design-system tokens', () => {
    // Three, because three is what the prototype states and `ds-sync` extracts. A fourth would
    // be a colour somebody invented, which is the act Principle I exists to prevent.
    const hues = Object.entries(ANSI_TOKENS).filter(([, token]) =>
      token.startsWith('--vk-term-ansi-'),
    );
    expect(hues.map(([slot]) => slot).sort()).toEqual(['green', 'red', 'yellow']);
  });

  it('maps black and white to structural tokens rather than dedicated ones', () => {
    // A dark theme's black **is** its background, and inventing a second near-black would be
    // inventing a colour. These reuse what the window already uses.
    expect(ANSI_TOKENS.black).toBe('--color-bg');
    expect(ANSI_TOKENS.white).toBe('--color-text');
  });

  it('defers eleven slots to the library, and names them', () => {
    // **Eleven, not thirteen.** A-TERMPALETTE's thirteen counts the colours with no token of
    // their own, which includes black and white; these eleven are the ones deferred. The two
    // numbers were stated as one, and the tests would have encoded eighteen of sixteen colours.
    expect(DEFERRED_ANSI.length).toBe(11);
    expect(Object.keys(ANSI_TOKENS).length + DEFERRED_ANSI.length).toBe(ANSI_SLOTS);
  });

  it('accounts for every ANSI slot exactly once', () => {
    // A slot in both lists would be mapped and deferred at the same time; a slot in neither
    // would be a colour nobody decided, which is how thirteen invented colours would arrive.
    const named = [...Object.keys(ANSI_TOKENS), ...DEFERRED_ANSI].sort();
    expect(new Set(named).size).toBe(ANSI_SLOTS);
    expect(named).toEqual([
      'black',
      'blue',
      'brightBlack',
      'brightBlue',
      'brightCyan',
      'brightGreen',
      'brightMagenta',
      'brightRed',
      'brightWhite',
      'brightYellow',
      'cyan',
      'green',
      'magenta',
      'red',
      'white',
      'yellow',
    ]);
  });

  it('takes the panel background, foreground and cursor from the window', () => {
    // What makes the panel look like part of the application instead of an embedded island. The
    // cursor is the prototype's own accent-coloured block, stated in its markup.
    expect(SURFACE_TOKENS.background).toBe('--color-bg');
    expect(SURFACE_TOKENS.foreground).toBe('--color-text');
    expect(SURFACE_TOKENS.cursor).toBe('--color-accent');
  });

  it('contains zero raw colour values of its own', () => {
    // The property SC-016 actually measures. Every value in this file is a token name; a hex
    // literal appearing anywhere in it is the failure, whatever it is called.
    const values = [...Object.values(ANSI_TOKENS), ...Object.values(SURFACE_TOKENS)];
    for (const value of values) {
      expect(value).toMatch(/^--[a-z-]+$/);
      expect(value).not.toMatch(/#[0-9a-fA-F]{3,8}/);
      expect(value).not.toMatch(/\brgb|hsl\b/);
    }
  });

  it('keeps the mapping in one place, which is what makes this checkable', () => {
    // Spread across the panel that mounts the terminal, SC-016 would need a reader to find every
    // site before they could say whether a colour had been invented.
    const everything = [
      ...Object.keys(ANSI_TOKENS),
      ...DEFERRED_ANSI,
      ...Object.keys(SURFACE_TOKENS),
    ];
    expect(everything.length).toBe(ANSI_SLOTS + 3);
  });
});
