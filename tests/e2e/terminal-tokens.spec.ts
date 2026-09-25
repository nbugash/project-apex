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
