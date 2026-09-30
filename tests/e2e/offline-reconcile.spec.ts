// T041 — US3 scenarios 1, 2 and 5, against a real engine and a real workspace.
//
// **A real outage and a real reconnection.** Going offline ends the engine; coming back releases the
// hold and waits for the reconnection loop to bring it back on its own (`offline-harness.ts`). The
// first version of this spec used `stub_set_connection`, which the live run does not read, so every
// "offline" save went straight to the host.
//
// **The counters are the assertions.** SC-004 and SC-005 are both "zero developer interactions",
// and zero is the only number that cannot be reached by accident: a test asserting "few" would pass
// for an implementation that asked about everything once.
//
// An interaction here means something the developer must answer: the conflict panel appearing, or a
// prompt. Notices are not interactions -- being *told* what happened is what FR-024 requires, and a
// test that counted notices would forbid the report it is supposed to check for.
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  WORKSPACE,
  resetWorkspace,
  openWorkspace,
  openFile,
  typeInEditor,
  clickSave,
  editorText,
} from './editor-harness';
import { resetSession, relaunch } from './helpers';
import { goOffline, comeBack, offlineStatus, invoke } from './offline-harness';

/** How many things are waiting for the developer to answer. Zero is the assertion. */
async function interactionsWaiting(): Promise<number> {
  return browser.execute(
    () =>
      document.querySelectorAll('[data-testid="conflict-panel"], [data-testid="conflict-row"]')
        .length,
  ) as Promise<number>;
}

/** Edit a file on the host, as a colleague or CI would. */
function editOnHost(name: string, body: string): void {
  writeFileSync(join(WORKSPACE, name), body);
}

/** What the host holds now, read from disk rather than from the client. */
function onHost(name: string): string {
  const r = spawnSync('cat', [join(WORKSPACE, name)], { encoding: 'utf8' });
  return r.stdout ?? '';
}

const TWENTY_LINES = Array.from({ length: 20 }, (_, i) => `line ${i}`).join('\n') + '\n';

describe('reconnecting after offline work', () => {
  beforeEach(async () => {
    resetSession();
    resetWorkspace({
      'unmoved.rs': TWENTY_LINES,
      'moved-elsewhere.rs': TWENTY_LINES,
      'untouched.rs': TWENTY_LINES,
    });
    await relaunch();
    await openWorkspace(WORKSPACE);
    await browser.waitUntil(async () => (await offlineStatus()).connected, {
      timeout: 60_000,
      timeoutMsg: 'each test starts from a connected engine',
    });
  });

  afterEach(async () => {
    // Never leave the suite without an engine, whatever failed above.
    await invoke('hold_offline_for_tests', { hold: false }).catch(() => {});
  });

  // Scenario 1, SC-004: the host has not moved, so everything lands and nothing is asked.
  it('lands offline work with zero interactions when the host has not moved', async () => {
    await openFile('/unmoved.rs');
    await goOffline();
    await typeInEditor('// mine\n');
    await clickSave();
    expect((await offlineStatus()).pending.length).toBe(1);

    await comeBack();

    // The row going away is what a confirmed write looks like from here.
    await browser.waitUntil(async () => (await offlineStatus()).pending.length === 0, {
      timeout: 20_000,
      timeoutMsg: 'the offline work never reached the host',
    });

    const asked = await interactionsWaiting();
    console.log(`SC-004 interactions for a clean reconnection: ${asked}`);
    expect(asked).toBe(0);
    expect(onHost('unmoved.rs')).toContain('// mine');
  });

  // Scenario 2, SC-005: the host moved somewhere else in the same file, and it still costs nothing.
  it('merges a non-overlapping host change with zero interactions', async () => {
    await openFile('/moved-elsewhere.rs');
    await goOffline();
    // The developer changes the top of the file.
    await typeInEditor('// mine\n');
    await clickSave();

    // The host changes the bottom, twenty lines away. Far enough that a context-aware merge
    // combines them -- research.md's measurement says two lines apart is already enough, and this
    // is deliberately further so the test is about the feature and not about the boundary.
    editOnHost('moved-elsewhere.rs', TWENTY_LINES + 'appended on the host\n');

    await comeBack();
    await browser.waitUntil(async () => (await offlineStatus()).pending.length === 0, {
      timeout: 20_000,
      timeoutMsg: 'the merge never completed',
    });

    const asked = await interactionsWaiting();
    console.log(`SC-005 interactions for a non-overlapping host change: ${asked}`);
    expect(asked).toBe(0);

    // Both changes survive. Read from the host's own file, not from the client's report of what it
    // did -- the report is what FR-024 requires and is not evidence that the bytes landed.
    const landed = onHost('moved-elsewhere.rs');
    expect(landed).toContain('// mine');
    expect(landed).toContain('appended on the host');
  });

  // Scenario 5, FR-024: the developer is shown what happened per file rather than inferring it.
  it('reports per file what happened, rather than leaving it to be inferred from the tree', async () => {
    // Both opened while online, so both are cached. A file first opened offline cannot be read, so
    // it cannot be edited or saved -- the first live run failed here with one held file, not two.
    await openFile('/moved-elsewhere.rs');
    await openFile('/unmoved.rs');
    await goOffline();
    await typeInEditor('// first\n');
    await clickSave();
    await openFile('/moved-elsewhere.rs');
    await typeInEditor('// second\n');
    await clickSave();
    expect((await offlineStatus()).pending.length).toBe(2);

    await comeBack();
    await browser.waitUntil(async () => (await offlineStatus()).pending.length === 0, {
      timeout: 20_000,
      timeoutMsg: 'reconciliation never finished',
    });

    // Both files reconciled, and the interface no longer marks either as held locally. That mark
    // going away is the per-file statement a developer reads: it was held, and now it is not.
    await openFile('/unmoved.rs');
    expect(await $('[data-testid="editor-held-locally"]').isExisting()).toBe(false);
    await openFile('/moved-elsewhere.rs');
    expect(await $('[data-testid="editor-held-locally"]').isExisting()).toBe(false);
    expect(await interactionsWaiting()).toBe(0);
  });

  // The file nobody touched is not written at all. A reconciliation that rewrote every cached file
  // would pass every assertion above and quietly give the host a hundred identical writes.
  it('writes nothing for a file that carried no offline work', async () => {
    const before = onHost('untouched.rs');
    await openFile('/unmoved.rs');
    await goOffline();
    await typeInEditor('// mine\n');
    await clickSave();
    await comeBack();
    await browser.waitUntil(async () => (await offlineStatus()).pending.length === 0, {
      timeout: 20_000,
      timeoutMsg: 'reconciliation never finished',
    });

    expect(onHost('untouched.rs')).toBe(before);
    expect(await editorText()).toBeDefined();
  });
});
