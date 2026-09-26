/**
 * The terminal's colours, and the single place any of them is decided.
 *
 * A terminal renders sixteen ANSI colours plus a default foreground, background and cursor.
 * The signed-off design system defines three of the sixteen (A-TERMPALETTE) and the structural
 * colours the rest of the window already uses. Everything else comes from the terminal library's
 * own palette, as a named exception rather than a silent one.
 *
 * **No colour is invented in this file.** That is the property SC-016 measures, and keeping the
 * whole mapping here -- rather than spreading it across the panel that mounts the terminal -- is
 * what makes it checkable by reading one file. A hex literal appearing below is the failure.
 *
 * The values are read from the mounted element rather than imported, because an `ITheme` takes
 * resolved colour strings and cannot take a `var()`. Nothing here is cached: `readPalette` resolves
 * at the moment it is called, so a palette cannot go stale relative to the window around it. That
 * is the structural half of "re-read when the theme changes"; `watchPalette` is the other half.
 */

import type { ITheme } from '@xterm/xterm';

/**
 * The non-ANSI slots: what the panel looks like when a program has said nothing about colour.
 *
 * These reuse the window's own structural tokens rather than terminal-specific ones, which is what
 * makes the panel look like part of the application instead of an embedded island. The cursor is
 * the prototype's own accent-coloured block -- stated in its markup, so taken from it.
 */
export const SURFACE_TOKENS = {
  background: '--color-bg',
  foreground: '--color-text',
  cursor: '--color-accent',
} as const;

/**
 * The five ANSI slots this design system can answer for, and the token each takes.
 *
 * Three are the hues the prototype states and `ds-sync` extracts. The other two are ANSI black and
 * white, which get the window's background and text rather than a dedicated token: a dark theme's
 * black *is* its background, and inventing a second near-black would be inventing a colour.
 *
 * A-TERMPALETTE counts thirteen colours as lacking a dedicated token, which includes these two.
 * That is a different quantity from the eleven deferred below, and the two numbers have been
 * confused once already.
 */
export const ANSI_TOKENS = {
  black: '--color-bg',
  red: '--vk-term-ansi-red',
  green: '--vk-term-ansi-green',
  yellow: '--vk-term-ansi-yellow',
  white: '--color-text',
} as const;

/**
 * The eleven ANSI slots the design system does not define, deferred to the library's palette.
 *
 * Named rather than merely omitted. An omission is indistinguishable from an oversight, and this
 * list is the visible marker A-TERMPALETTE relies on to say a decision is still outstanding -- the
 * design system owes a sixteen-colour ramp, and when it arrives this array empties.
 *
 * It is also what lets the rendered half of SC-016 assert these equal the library's default instead
 * of leaving them unasserted, so a hand-typed hex that happens to look right still fails.
 */
export const DEFERRED_ANSI = [
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
] as const;

/** Resolve every mapped slot against `el`'s computed style. */
export function readPalette(el: Element): ITheme {
  const computed = getComputedStyle(el);
  const theme: Record<string, string> = {};
  for (const [slot, token] of [...Object.entries(SURFACE_TOKENS), ...Object.entries(ANSI_TOKENS)]) {
    const value = computed.getPropertyValue(token).trim();
    // An unresolved token means the stylesheet has not loaded or `ds:sync` did not run. Leaving
    // the slot out degrades to the library's colour, which keeps a readable panel on screen; the
    // absence is caught loudly by the unit and end-to-end tests rather than quietly at runtime,
    // because a blank terminal helps nobody and a missing token is a build fault, not a user's.
    if (value) theme[slot] = value;
  }
  return theme as ITheme;
}

/**
 * Re-resolve the palette whenever the document's theme could have changed, and hand it to `apply`.
 *
 * Built on the platform's own signals -- an attribute change on the root element, and the system
 * colour-scheme media query -- rather than on an application event, so it needs no cooperation from
 * whatever eventually introduces theme switching. The design system currently ships a single theme,
 * so neither signal fires today; the watcher is correct and quiet rather than absent and owed.
 *
 * Returns the unsubscribe function. A panel that forgets to call it holds its element alive.
 */
export function watchPalette(el: Element, apply: (theme: ITheme) => void): () => void {
  const reread = () => apply(readPalette(el));
  const attributes = new MutationObserver(reread);
  attributes.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class', 'style', 'data-theme'],
  });
  const scheme = window.matchMedia('(prefers-color-scheme: light)');
  scheme.addEventListener('change', reread);
  return () => {
    attributes.disconnect();
    scheme.removeEventListener('change', reread);
  };
}
