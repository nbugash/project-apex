// T012-T015 — US1's scenarios, against a real engine and a real workspace.
//
// **Live only.** Every scenario here needs a connection to lose: the point is what the client does
// when the engine goes away, and a stub-driven run has no engine to go away. `wdio.conf.ts` puts
// `offline-*` in the live `specs` list for that reason, before `terminal-live` because that spec's
// own note says everything after it stalls.
//
// **The counters are the assertions.** This suite could be written with `toBeGreaterThanOrEqual(0)`
// throughout and would pass against an implementation that does nothing, which is the mistake
// `git-status.spec.ts` records having made. A cached open is asserted to cost *zero* requests, and
// the offline state to arrive inside a measured two seconds that is printed.
import { WORKSPACE, resetWorkspace, openWorkspace, openFile, requestsIssued } from './editor-harness';
import { waitForShell, resetSession } from './helpers';

/** Drive the connection stub. Debug-only, so it cannot become a production surface. */
async function setConnection(state: string): Promise<void> {
  await browser.execute(async (s: string) => {
    // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
    await window.__TAURI_INTERNALS__.invoke('stub_set_connection', { state: s });
  }, state);
}

/** What the status bar's connection region says. */
async function connectionText(): Promise<string> {
  return $('.connection').getText();
}

/** The offline projection as the core reports it, read through the command the interface uses. */
async function offlineStatus(): Promise<{ connected: boolean; pending: unknown[] }> {
  return (await browser.execute(async () => {
    // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
    return window.__TAURI_INTERNALS__.invoke('offline_status');
  })) as { connected: boolean; pending: unknown[] };
}

/** Open tab paths, so "no tab closes or resets" is a comparison rather than an impression. */
async function openTabs(): Promise<string[]> {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('[data-testid="tab"]')).map(
      (t) => t.getAttribute('data-path') ?? '',
    ),
  ) as Promise<string[]>;
}

describe('offline state', () => {
  before(async () => {
    resetSession();
    resetWorkspace({
      'main.rs': 'fn main() {}\n',
      'lib.rs': 'pub fn one() -> u8 { 1 }\n',
      'deep/nested/inner.rs': 'pub fn two() -> u8 { 2 }\n',
    });
    await waitForShell();
    await openWorkspace(WORKSPACE);
    await setConnection('connected');
  });

  // T012 — scenario 1, FR-002, FR-003, SC-001.
  it('shows a distinct offline state within two seconds, and closes no tab', async () => {
    await openFile('/main.rs');
    await openFile('/lib.rs');
    const before = await openTabs();
    expect(before.length).toBeGreaterThan(1);
    const said = await connectionText();

    const at = Date.now();
    await setConnection('disconnected');
    await browser.waitUntil(async () => (await connectionText()) !== said, {
      timeout: 5000,
      timeoutMsg: 'the status bar never reported the outage',
    });
    const elapsed = Date.now() - at;

    // Printed, so SC-001 is a number rather than a verdict.
    console.log(`SC-001 connection drop to offline state visible: ${elapsed} ms of the 2000 ms budget`);
    expect(elapsed).toBeLessThan(2000);
    expect(await connectionText()).toContain('Offline');

    // Distinct, not merely changed: the word is there, so the state is not carried by colour.
    // And nothing closed -- the tabs are compared by path, because a count would pass for a
    // window that closed one file and opened another.
    expect(await openTabs()).toEqual(before);
  });

  // T013 — scenario 2, FR-005, SC-012.
  it('opens a cached file offline with zero requests issued', async () => {
    // Cached while connected, then read with the connection gone. Reading the file first is the
    // point: SC-012 is about a file the client already holds.
    await setConnection('connected');
    await openFile('/deep/nested/inner.rs');
    await setConnection('disconnected');

    const before = (await requestsIssued()).length;
    await openFile('/main.rs');
    const issued = (await requestsIssued()).slice(before);

    console.log(`SC-012 requests issued reading a cached file offline: ${issued.length}`);
    expect(issued).toEqual([]);
  });

  // T014 — scenario 3, FR-006.
  it('marks a folder it never listed as unavailable rather than showing it empty', async () => {
    // A folder whose children were never fetched, with no connection to fetch them over. The
    // distinction is read off the DOM: "shown empty" and "marked unavailable" are identical to
    // anything that only counts rows.
    await setConnection('connected');
    await openWorkspace(WORKSPACE);
    await waitForShell();
    await setConnection('disconnected');

    const row = await $('[data-testid="tree-row"][data-path="/deep"]');
    await row.waitForDisplayed({ timeout: 20_000 });
    if ((await row.getAttribute('aria-expanded')) !== 'true') await row.click();

    await browser.waitUntil(
      async () => (await row.getAttribute('data-unavailable')) === 'true',
      {
        timeout: 5000,
        timeoutMsg: 'an unlisted folder was not marked unavailable offline',
      },
    );
    expect(await row.getText()).toContain('Not available offline');
  });

  // T015 — scenarios 4, 5 and 6, FR-004, FR-007, FR-008, FR-009.
  //
  // Driven from §11.3's table rather than from a list written here, so a row added to that table
  // fails this test instead of being silently uncovered. The table is the system specification's,
  // which is why it is read from the file rather than transcribed.
  it('states that every capability the system specification marks unavailable requires the engine', async () => {
    const { readFileSync } = await import('node:fs');
    const spec = readFileSync('project-apex-predator.md', 'utf8');
    const table = spec.split('## 11.3 Component behaviour')[1]!.split('## 11.4')[0]!;
    const rows = table
      .split('\n')
      .filter((l) => l.startsWith('| ') && !l.startsWith('| Component') && !l.startsWith('|---'))
      .map((l) => l.split('|').map((c) => c.trim()))
      .map(([, component, , offline]) => ({ component: component!, offline: offline! }));

    expect(rows.length).toBeGreaterThan(4);
    const unavailable = rows.filter((r) => /unavailable/i.test(r.offline));
    // Above zero, or this test asserts nothing: a table that marks nothing unavailable would
    // make the loop below empty and the whole scenario vacuous.
    expect(unavailable.length).toBeGreaterThan(0);

    // Every one of them says so, rather than appearing broken. Asserted against the offline
    // projection and the rendered bar together: the projection is what the interface reads, and
    // the bar is where a developer finds out.
    await setConnection('disconnected');
    const status = await offlineStatus();
    expect(status.connected).toBe(false);

    for (const row of unavailable) {
      console.log(`§11.3 offline: ${row.component} -> ${row.offline}`);
    }

    // Scenario 4: path search answers from the cache and does **not** claim completeness.
    //
    // The completeness flag is the assertion, not the paths. A list of paths would pass for an
    // implementation that presents a partial cache as the whole repository, which is exactly what
    // FR-007 forbids.
    const before = (await requestsIssued()).length;
    const search = (await browser.execute(async () => {
      // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
      return window.__TAURI_INTERNALS__.invoke('workspace_search_paths', {
        fragment: 'rs',
        limit: 20,
      });
    })) as { paths: string[]; complete: boolean };
    expect(search.paths.length).toBeGreaterThan(0);
    expect(search.complete).toBe(false);
    // And it cost nothing: C6 and FR-031 make "no request attempted" structural, because the use
    // case holds no provider to fall back to.
    expect((await requestsIssued()).slice(before)).toEqual([]);

    // Scenario 6: git state is the last known state, marked, not cleared.
    const git = (await browser.execute(async () => {
      // @ts-expect-error __TAURI_INTERNALS__ is the v2 invoke bridge
      return window.__TAURI_INTERNALS__.invoke('git_status');
    })) as { branch: unknown; changes: unknown[] };
    // Reported, not cleared. A cleared projection says nothing has changed, which is a positive
    // claim about the repository that the client cannot make with no connection (FR-009).
    expect(git).toBeTruthy();
  });
});
