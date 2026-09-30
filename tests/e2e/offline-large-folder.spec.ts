// F005's FR-024 through the real interface: a folder larger than one page (1000 entries) reaches the
// tree whole, and is whole offline too.
//
// Found after F012 by reading, not by a report. The tree asked for the first page and dropped the
// cursor, and the caching layer stored that page as though it were the folder, so a folder of more
// than 1000 entries showed 1000, online and offline alike, with nothing to say the rest existed.
//
// Named `offline-*` because the offline half is the point as much as the online one -- a cached
// folder must be the whole folder or nothing -- and so that the live suite's pattern runs it.
import { join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { WORKSPACE, resetWorkspace, openWorkspace } from './editor-harness';
import { resetSession, relaunch } from './helpers';
import {
  goOffline,
  comeBack,
  offlineStatus,
  invoke,
  holdNextLaunchOffline,
  releaseNextLaunch,
} from './offline-harness';

const ENTRIES = 1_500;

async function rowsUnder(prefix: string): Promise<number> {
  return browser.execute(
    (p: string) =>
      Array.from(document.querySelectorAll('[data-testid="tree-row"]')).filter((r) =>
        (r.getAttribute('data-path') ?? '').startsWith(p),
      ).length,
    prefix,
  );
}

describe('a folder larger than one page', () => {
  before(async () => {
    resetSession();
    resetWorkspace({ 'README.md': '# big\n' });
    mkdirSync(join(WORKSPACE, 'big'), { recursive: true });
    for (let i = 0; i < ENTRIES; i += 1) {
      writeFileSync(join(WORKSPACE, 'big', `f${String(i).padStart(5, '0')}.rs`), 'x\n');
    }
    await relaunch();
    await openWorkspace(WORKSPACE);
    await browser.waitUntil(async () => (await offlineStatus()).connected, {
      timeout: 60_000,
      timeoutMsg: 'the test starts from a connected engine',
    });
  });

  after(async () => {
    releaseNextLaunch();
    await invoke('hold_offline_for_tests', { hold: false }).catch(() => {});
  });

  it('shows every entry, online and then offline', async () => {
    const folder = await $('[data-testid="tree-row"][data-path="/big"]');
    await folder.waitForDisplayed({ timeout: 20_000 });
    await folder.click();
    await browser.waitUntil(async () => (await rowsUnder('/big/')) === ENTRIES, {
      timeout: 30_000,
      timeoutMsg: `the tree did not show all ${ENTRIES} entries`,
    });

    // Offline, after a relaunch, so what is shown is what the cache holds, not what the view kept.
    // The hold file makes the relaunched process start offline too; the session restores the
    // workspace, so nothing here asks a host that is not there.
    await goOffline();
    holdNextLaunchOffline();
    await relaunch();
    const again = await $('[data-testid="tree-row"][data-path="/big"]');
    await again.waitForDisplayed({ timeout: 20_000 });
    if ((await again.getAttribute('aria-expanded')) !== 'true') await again.click();
    await browser.waitUntil(async () => (await rowsUnder('/big/')) === ENTRIES, {
      timeout: 30_000,
      timeoutMsg: `offline, the cached folder did not hold all ${ENTRIES} entries`,
    });
    expect((await offlineStatus()).connected).toBe(false);
    releaseNextLaunch();
    await comeBack();
  });
});
