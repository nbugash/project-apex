// Helpers for the live offline specs (F012).
//
// **The outage is always real.** In the live run `stub_set_connection` drives a source nothing
// reads -- `editor-save.spec.ts` records that -- so going offline means ending the engine. The
// reconnection loop would bring it back within a second, so the specs *hold* reconnection off while
// they work inside the outage, through a debug-only command compiled out of release builds, and
// release the hold to let the real loop reconnect on its own schedule.
import { spawnSync } from 'node:child_process';
import { rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { dataDir } from './helpers';

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return (await browser.execute(
    async (c: string, a: Record<string, unknown>) => {
      // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
      return window.__TAURI_INTERNALS__.invoke(c, a);
    },
    cmd,
    args,
  )) as T;
}

/**
 * End the engine and wait until it is really gone.
 *
 * `pkill` only delivers a signal. Returning while the engine was still exiting once left the kill
 * racing the next spec file's startup (`editor-session.spec.ts` failed its setup in a full run).
 * The blast radius is every engine on the machine, tolerable only because the suite runs one
 * application at a time (`maxInstances: 1`).
 */
export function killEngine(): void {
  spawnSync('pkill', ['-x', 'ide-engine']);
  for (let i = 0; i < 50; i += 1) {
    if (spawnSync('pgrep', ['-x', 'ide-engine']).status !== 0) return;
    spawnSync('sleep', ['0.1']);
  }
  throw new Error('the engine would not stop, so the test that follows cannot trust its state');
}

export interface OfflineStatus {
  connected: boolean;
  pending: Array<{ relativePath: string; mergeable: boolean }>;
}

export async function offlineStatus(): Promise<OfflineStatus> {
  return invoke<OfflineStatus>('offline_status');
}

/** What the status bar's connection region says. */
export async function connectionText(): Promise<string> {
  return $('.connection').getText();
}

/** Go offline for real, and stay there until `comeBack`. */
export async function goOffline(): Promise<void> {
  await invoke('hold_offline_for_tests', { hold: true });
  killEngine();
  await browser.waitUntil(async () => !(await offlineStatus()).connected, {
    timeout: 10_000,
    timeoutMsg: 'the client never noticed the engine had gone',
  });
}

/** Release the hold and wait for the reconnection loop to bring the engine back. */
export async function comeBack(): Promise<void> {
  await invoke('hold_offline_for_tests', { hold: false });
  // Up to the loop's own ceiling and then some: after a few refused attempts the next wait can be
  // as long as the backoff allows, and the loop is what is being exercised here.
  await browser.waitUntil(async () => (await offlineStatus()).connected, {
    timeout: 60_000,
    timeoutMsg: 'the reconnection loop never brought the engine back',
  });
}

/** The hold file that makes the *next* launch start offline (debug builds only). */
const HOLD_FILE = (): string => join(dataDir(), 'hold-offline');

export function holdNextLaunchOffline(): void {
  writeFileSync(HOLD_FILE(), '');
}

export function releaseNextLaunch(): void {
  rmSync(HOLD_FILE(), { force: true });
}

/** Open tab identities in order, so "no tab closes" is a comparison rather than an impression. */
export async function openTabs(): Promise<string[]> {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('.strip [role="tab"]')).map(
      (t) => t.getAttribute('data-tab') ?? '',
    ),
  ) as Promise<string[]>;
}
