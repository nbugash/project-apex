// T053 — US4 scenario 4, FR-025 and SC-003, against a real engine and a real workspace.
//
// **The full cycle, with every outage real.** Edit offline, collide with the host, reconnect and be
// asked; then go offline *again* with the conflict unresolved, quit, relaunch still offline, and
// reconnect -- and be asked again, with nothing written and nothing lost. The second disconnection
// is a real one (`goOffline` ends the engine) and the conflict is genuinely unresolved throughout,
// not resolved and re-created: the host's file is read from disk at every step and never changes
// until the developer answers.
//
// Then the developer answers, and the draft is refused while it still carries markers (FR-033):
// the client writes a combination only when somebody chose it.
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
import { resetSession, relaunch, waitForShell } from './helpers';
import {
  goOffline,
  comeBack,
  offlineStatus,
  invoke,
  holdNextLaunchOffline,
  releaseNextLaunch,
} from './offline-harness';

const TWENTY_LINES = Array.from({ length: 20 }, (_, i) => `line ${i}`).join('\n') + '\n';
const THEIRS = '// theirs\n' + TWENTY_LINES;

function onHost(name: string): string {
  return spawnSync('cat', [join(WORKSPACE, name)], { encoding: 'utf8' }).stdout ?? '';
}

async function panelShown(why: string): Promise<void> {
  await $('[data-testid="conflict-panel"]').waitForDisplayed({ timeout: 30_000, timeoutMsg: why });
}

/** Set the result box as a developer's edit would, through the element's own input event. */
async function editResult(text: string): Promise<void> {
  await browser.execute((t: string) => {
    const box = document.querySelector<HTMLTextAreaElement>('[data-testid="conflict-result"]');
    if (!box) throw new Error('no result box');
    box.value = t;
    box.dispatchEvent(new Event('input', { bubbles: true }));
  }, text);
}

describe('a conflict the developer decides', () => {
  beforeEach(async () => {
    releaseNextLaunch();
    resetSession();
    resetWorkspace({ 'shared.rs': TWENTY_LINES });
    await relaunch();
    await openWorkspace(WORKSPACE);
    await browser.waitUntil(async () => (await offlineStatus()).connected, {
      timeout: 60_000,
      timeoutMsg: 'each test starts from a connected engine',
    });
  });

  afterEach(async () => {
    releaseNextLaunch();
    await invoke('hold_offline_for_tests', { hold: false }).catch(() => {});
  });

  it('survives going offline and a relaunch unresolved, loses nothing, and is asked again', async () => {
    // Offline, the developer adds a line at the top; the host adds a different one at the same
    // place. Insert against insert at one point is a conflict on git's terms.
    await openFile('/shared.rs');
    // Waited for: `openFile` returns when the editor exists, which can be before its model has
    // content, and typing into an empty model saves a different file than the one intended.
    await browser.waitUntil(async () => (await editorText()).includes('line 19'), {
      timeout: 10_000,
      timeoutMsg: 'the file never arrived',
    });
    await goOffline();
    await typeInEditor('// mine\n');
    await clickSave();
    await browser.waitUntil(async () => (await offlineStatus()).pending.length === 1, {
      timeout: 10_000,
      timeoutMsg: 'the offline save was not held',
    });
    writeFileSync(join(WORKSPACE, 'shared.rs'), THEIRS);

    // Scenario 1: asked, and neither version written.
    await comeBack();
    await panelShown('the developer was never asked about the collision');
    expect(onHost('shared.rs')).toBe(THEIRS);
    expect((await offlineStatus()).pending.length).toBe(1);

    // Scenario 4: offline again, unresolved, and a relaunch that is still offline.
    await goOffline();
    holdNextLaunchOffline();
    await relaunch();
    await waitForShell();
    await browser.waitUntil(
      async () => {
        try {
          return (await offlineStatus()).pending.length === 1;
        } catch {
          return false;
        }
      },
      { timeout: 20_000, timeoutMsg: 'the unresolved work did not survive the relaunch' },
    );
    expect(onHost('shared.rs')).toBe(THEIRS);

    // Back online: presented again, still nothing written.
    releaseNextLaunch();
    await comeBack();
    await panelShown('the unresolved conflict was not presented again after reconnecting');
    expect(onHost('shared.rs')).toBe(THEIRS);
    const kept = (await offlineStatus()).pending.length;
    console.log(`SC-003 offline work kept across edit, disconnect, relaunch, reconnect: ${kept} of 1`);
    expect(kept).toBe(1);

    // All three sides are on screen, each labelled.
    const sides = await $$('[data-testid="conflict-side"]');
    expect(sides.length).toBe(3);

    // FR-033: the draft as it stands still carries markers, and is refused.
    await $('[data-testid="conflict-use-result"]').click();
    const refusal = await $('[data-testid="conflict-message"]');
    await refusal.waitForDisplayed({ timeout: 10_000 });
    expect(await refusal.getText()).toMatch(/marker/i);
    expect(onHost('shared.rs')).toBe(THEIRS);

    // Scenario 3: the developer settles it, and exactly that is written.
    const resolved = '// mine\n// theirs\n' + TWENTY_LINES;
    await editResult(resolved);
    await $('[data-testid="conflict-use-result"]').click();
    await $('[data-testid="conflict-panel"]').waitForExist({ reverse: true, timeout: 10_000 });
    expect(onHost('shared.rs')).toBe(resolved);
    await browser.waitUntil(async () => (await offlineStatus()).pending.length === 0, {
      timeout: 10_000,
      timeoutMsg: 'the file is still marked as held locally after resolving',
    });
  });
});
