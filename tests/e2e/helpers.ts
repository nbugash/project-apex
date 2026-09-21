// Shared helpers for the end-to-end suite.
import { readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';

/** The profile directory for the run. Computed rather than read from the environment:
 *  WDIO workers are separate processes and do not inherit what onPrepare set. */
export const dataDir = (): string => join(process.cwd(), '.e2e-profile');
export const sessionPath = (): string => join(dataDir(), 'session.json');

export function readSession(): Record<string, unknown> | null {
  const p = sessionPath();
  return existsSync(p) ? JSON.parse(readFileSync(p, 'utf8')) : null;
}

export function writeSessionRaw(body: string): void {
  writeFileSync(sessionPath(), body);
}

/** Discard persisted state so a spec starts from defaults. One profile directory is shared
 *  across the run — the app inherits its environment from the single driver process — so
 *  isolation is explicit here rather than implicit in the harness. */
export function resetSession(): void {
  rmSync(sessionPath(), { force: true });
}

/** Restart the application so persisted state is read afresh. */
export async function relaunch(): Promise<void> {
  await browser.reloadSession();
  await waitForShell();
}

export async function waitForShell(): Promise<void> {
  await $('.shell').waitForExist({ timeout: 20_000 });

  // The DOM exists while the window is still hidden — that is the whole point of the
  // readiness gate. A hidden window reports a placeholder size, so wait for it to be
  // mapped at its real size before asserting anything visual or capturing a screenshot.
  await browser.waitUntil(async () => (await browser.execute(() => window.innerWidth)) > 400, {
    timeout: 20_000,
    timeoutMsg: 'window never became visible',
  });
}

/** Resolve a CSS custom property from the running document. */
export async function token(name: string): Promise<string> {
  return browser.execute(
    (n: string) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(),
    name,
  );
}

export async function openDocuments(count: number): Promise<void> {
  const button = await $('nav.placeholder button');
  for (let i = 0; i < count; i++) await button.click();
}
