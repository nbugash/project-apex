/**
 * The editor's colours, and the only place any of them is decided.
 *
 * Monaco takes a theme of colour strings; the design system supplies CSS custom properties. This
 * translates one into the other, reading the tokens off the mounted element so the stylesheet
 * stays the source and no colour is written in a component.
 *
 * # The mapping is this feature's; the values are not
 *
 * The prototype's editor writes its colours as hex literals, and every one of them is a
 * design-system value: `#9397ab` is `--color-neutral-500`, `#b5abfc` is `--color-accent-400`,
 * and so on. What this feature decides is which *role* takes which token, and the prototype
 * fixes that by using those colours for those roles. See A-EDITPALETTE, which records the
 * correction to an earlier reading that took the literals for raw hex outside the system.
 *
 * # Five roles, and the rest deferred
 *
 * Five is what the prototype distinguishes, not a target. Monaco knows about forty-odd more, and
 * inventing colours for them would be this application deciding what the design system looks
 * like. They fall back to the foreground: unstyled, which is honest, rather than mis-styled.
 */

/// Monaco's shape, named here so the module does not import Monaco's types — the theme is data
/// and this file is on the boundary where a library type would otherwise leak inward.
export interface EditorTheme {
  base: 'vs-dark';
  inherit: boolean;
  rules: Array<{ token: string; foreground: string }>;
  colors: Record<string, string>;
}

/// Syntax role to design-system token. The prototype's editor is the authority for each pairing.
export const SYNTAX_TOKENS: ReadonlyArray<readonly [role: string, token: string]> = [
  ['comment', '--color-neutral-600'],
  ['delimiter', '--color-neutral-500'],
  ['operator', '--color-neutral-500'],
  ['keyword', '--color-accent-400'],
  ['type', '--color-accent-300'],
  ['type.identifier', '--color-accent-300'],
  ['identifier', '--color-neutral-200'],
];

/// The surface, which is not syntax: these come from the same tokens the rest of the window uses,
/// so the editor is part of the application rather than an embedded island.
const SURFACE_TOKENS: ReadonlyArray<readonly [key: string, token: string]> = [
  ['editor.background', '--color-bg'],
  ['editor.foreground', '--color-text'],
  ['editorCursor.foreground', '--color-accent'],
  ['editorLineNumber.foreground', '--color-neutral-600'],
  ['editorLineNumber.activeForeground', '--color-neutral-400'],
  ['editor.lineHighlightBackground', '--color-surface'],
  ['editor.selectionBackground', '--color-accent-800'],
  ['editorIndentGuide.background1', '--color-divider'],
];

function read(computed: CSSStyleDeclaration, token: string): string | null {
  const value = computed.getPropertyValue(token).trim();
  return value === '' ? null : value;
}

/**
 * Build the theme from the tokens on `el`.
 *
 * A token the stylesheet does not define is **omitted** rather than defaulted. Monaco then uses
 * its own value, which is visibly wrong and therefore gets fixed; a silent default is invisibly
 * wrong and stays.
 */
export function editorTheme(el: HTMLElement): EditorTheme {
  const computed = getComputedStyle(el);
  const rules: EditorTheme['rules'] = [];
  for (const [role, token] of SYNTAX_TOKENS) {
    const value = read(computed, token);
    // Monaco wants `RRGGBB` without the hash in `rules`, and `#RRGGBB` in `colors`. One of the
    // few places this file has to know something about the library it is feeding.
    if (value) rules.push({ token: role, foreground: value.replace(/^#/, '') });
  }
  const colors: Record<string, string> = {};
  for (const [key, token] of SURFACE_TOKENS) {
    const value = read(computed, token);
    if (value) colors[key] = value;
  }
  return { base: 'vs-dark', inherit: true, rules, colors };
}
