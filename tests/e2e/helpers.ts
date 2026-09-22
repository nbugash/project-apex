// Shared helpers for the end-to-end suite.
import { readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
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

/** Block until the session file on disk satisfies a predicate.
 *
 *  Writes are debounced by 250 ms in the core, and restarting the application does not
 *  flush them — so relaunching immediately after a command loses it, and the test then
 *  fails somewhere unrelated to what it was checking. Waiting on the file is a condition,
 *  not a sleep: it cannot pass early and does not get slower on a loaded machine. */
export async function waitForPersisted(
  predicate: (session: Record<string, unknown>) => boolean,
  what: string,
): Promise<void> {
  await browser.waitUntil(
    async () => {
      const s = readSession();
      return s !== null && predicate(s);
    },
    { timeout: 10_000, timeoutMsg: `${what} was never persisted` },
  );
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

/** Minimum distinct colours a capture of a rendered shell contains.
 *
 *  A blank display yields exactly one. The shell yields several hundred. The threshold sits
 *  far from both, so it cannot fail on rendering variation between machines while still
 *  catching the case it exists for. */
const MINIMUM_COLOURS = 16;

/** Fail loudly when a screenshot shows nothing.
 *
 *  This is the check F000 did not have. The application shipped a window that was never
 *  made visible: the interface waited for two animation frames before signalling readiness,
 *  and a hidden window produces no animation frames. Every assertion in the suite passed —
 *  WebDriver drives a hidden window perfectly well — and every screenshot was a black
 *  rectangle. It was found by a person opening an image, which is not a control.
 *
 *  A capture that cannot be inspected is reported rather than thrown, because a broken
 *  capture tool should not be indistinguishable from a broken application. */
export function assertCaptureIsNotBlank(file: string): string | null {
  if (!existsSync(file)) return `no screenshot was written to ${file}`;

  const probe = spawnSync('identify', ['-format', '%k', file], {
    encoding: 'utf8',
    timeout: 10_000,
  });
  if (probe.error || probe.status !== 0) return null; // cannot inspect; not evidence of a defect

  const colours = Number.parseInt((probe.stdout || '').trim(), 10);
  if (!Number.isFinite(colours)) return null;
  if (colours < MINIMUM_COLOURS) {
    return (
      `the screenshot ${file} has ${colours} distinct colour(s), which means the window ` +
      'was never shown. The suite can pass against a hidden window; a user cannot use one.'
    );
  }
  return null;
}

/** Resolve a CSS custom property from the running document. */
export async function token(name: string): Promise<string> {
  return browser.execute(
    (n: string) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(),
    name,
  );
}

/** Set a region's visibility or extent through the core.
 *
 *  Like openDocuments, this replaces a click on F018-removed scaffolding. The prototype
 *  controls the bottom dock from its own tab bar, which is a later feature. */
export async function setRegion(
  region: 'output' | 'document_area',
  visible: boolean,
  extent: number,
): Promise<void> {
  await browser.execute(
    async (r: string, v: boolean, e: number) => {
      // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
      await window.__TAURI_INTERNALS__.invoke('layout_set_region', {
        region: r,
        visible: v,
        extent: e,
      });
    },
    region,
    visible,
    extent,
  );
  await waitForPersisted(
    (s) =>
      ((s.layout as Record<string, { visible: boolean }>)?.[region]?.visible ?? null) === visible,
    `${region} visibility`,
  );
  await relaunch();
}

/** Open N documents.
 *
 *  Drives the core command rather than a button. F000 opened documents from a "New
 *  document" control in the navigation placeholder; F018 replaced that placeholder with
 *  the prototype's tool window, which has no such control — the prototype opens documents
 *  from the file tree, which is a later feature. Going through the command keeps these
 *  tests testing tab behaviour instead of scaffolding that was never meant to ship.
 *
 *  The relaunch is what makes the new documents visible: the interface renders from the
 *  session it read at startup, and a command invoked from outside it does not notify it.
 *  Restarting also flushes the debounced write, so this does not race the persistence
 *  timer the way a fixed pause would. */
export async function openDocuments(count: number): Promise<void> {
  await browser.execute(async (n: number) => {
    for (let i = 0; i < n; i++) {
      // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
      await window.__TAURI_INTERNALS__.invoke('documents_open', {
        displayName: `untitled-${i + 1}`,
      });
    }
  }, count);
  await waitForPersisted(
    (s) => (s.documents as unknown[] | undefined)?.length === count,
    `${count} documents`,
  );
  await relaunch();
}
