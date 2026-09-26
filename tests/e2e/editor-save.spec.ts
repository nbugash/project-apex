// T043 — US2's acceptance scenarios, against a real engine and a real directory.
//
// **Every assertion about a write reads the file back off disk.** A spec that checked what the
// interface said would pass for an application that reported a conflict and wrote anyway, which
// is the one defect the whole base-hash mechanism exists to prevent.
//
// The colleague here is `writeFileSync` from the test process. That is not a simulation of a
// colleague editing the file: it is a colleague editing the file.
import {
  resetWorkspace,
  openWorkspace,
  openFile,
  editorText,
  typeInEditor,
  clickSave,
  onDisk,
  editOnHost,
  requestsIssued,
} from './editor-harness';
import { spawnSync } from 'node:child_process';
import { waitForShell, resetSession } from './helpers';

async function noticeTone(): Promise<string> {
  const notice = await $('[data-testid="editor-ending"]');
  await notice.waitForDisplayed({ timeout: 20_000 });
  return (await notice.getAttribute('data-tone')) ?? '';
}

/// End the engine the application spawned, which is what an outage is.
///
/// **Waits for the process to be gone before returning.** `pkill` only delivers a signal; it
/// says nothing about when the process dies. Returning while the engine was still exiting left
/// the kill racing the *next* spec file's startup, and `editor-session.spec.ts` failed its setup
/// once in a full run because of it — an engine that was reachable when its app asked for a
/// workspace and gone a moment later. The blast radius is still every engine on the machine,
/// which is tolerable only because the suite runs one application at a time (`maxInstances: 1`).
function killEngine(): void {
  spawnSync('pkill', ['-x', 'ide-engine']);
  for (let i = 0; i < 50; i += 1) {
    if (spawnSync('pgrep', ['-x', 'ide-engine']).status !== 0) return;
    spawnSync('sleep', ['0.1']);
  }
  throw new Error('the engine would not die, so the test that follows cannot trust its state');
}

describe('saving', () => {
  beforeEach(async () => {
    resetWorkspace({ 'notes.md': 'original\n' });
    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
    await openFile('/notes.md');
    await browser.waitUntil(async () => (await editorText()).includes('original'), {
      timeout: 20_000,
      timeoutMsg: 'the file never arrived',
    });
  });

  it('writes what is in the buffer to the host', async () => {
    await typeInEditor('edited ');
    await clickSave();
    await browser.waitUntil(async () => onDisk('notes.md').includes('edited'), {
      timeout: 20_000,
      timeoutMsg: 'nothing reached the disk',
    });
    expect(onDisk('notes.md')).toBe('edited original\n');
    expect(await noticeTone()).toBe('ok');
  });

  it('shows what was saved when the file is opened again', async () => {
    // FR-010. The projection has to hold what was written, or a reopen shows the old content
    // and the developer believes their save was lost.
    await typeInEditor('kept ');
    await clickSave();
    await browser.waitUntil(async () => onDisk('notes.md').includes('kept'), { timeout: 20_000 });

    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
    await openFile('/notes.md');
    await browser.waitUntil(async () => (await editorText()).includes('kept'), {
      timeout: 20_000,
      timeoutMsg: 'the reopened file did not show what was saved',
    });
  });

  it('refuses a save whose file moved underneath it, and writes nothing', async () => {
    await typeInEditor('mine ');
    // A colleague, between the read and the save.
    editOnHost('notes.md', 'theirs\n');

    await clickSave();

    expect(await noticeTone()).toBe('warning');
    // The assertion that matters. Everything else here would pass for an engine that reported
    // a conflict and overwrote the file anyway.
    expect(onDisk('notes.md')).toBe('theirs\n');
    // And the developer's work is still in front of them (FR-011).
    expect(await editorText()).toContain('mine');
  });

  it('offers exactly one way out of a conflict, and it is not overwriting', async () => {
    // SC-015, FR-012a and FR-012b. Discarding is the escape; there is deliberately no control
    // that writes over the host, because doing so destroys a colleague's work silently.
    await typeInEditor('mine ');
    editOnHost('notes.md', 'theirs\n');
    await clickSave();
    await noticeTone();

    const words = await browser.execute(
      () => document.querySelector('[data-testid="editor-ending"]')?.textContent ?? '',
    );
    expect(String(words).toLowerCase()).not.toMatch(/overwrite|force|anyway/);

    await (await $('[data-testid="editor-discard"]')).click();

    await browser.waitUntil(async () => (await editorText()) === 'theirs\n', {
      timeout: 20_000,
      timeoutMsg: 'discarding did not leave the host version in the buffer',
    });
    // Exactly the host's bytes, not a merge and not an approximation.
    expect(await editorText()).toBe('theirs\n');
    expect(onDisk('notes.md')).toBe('theirs\n');
  });

  it('reports an unreachable engine as the link, and keeps the work', async () => {
    // **Last in the file, and it ends the engine.** `stub_set_connection` cannot serve here:
    // with an engine present the composition root binds the real transport as the connection
    // source, so the stub drives something nothing is reading (see `wdio.conf.ts`). The honest
    // way to test "the engine could not be reached" is for it not to be reachable.
    await typeInEditor('offline ');
    killEngine();
    await browser.pause(1000);

    await clickSave();

    expect(await noticeTone()).toBe('error');
    const words = await browser.execute(
      () => document.querySelector('[data-testid="editor-ending"]')?.textContent ?? '',
    );
    // The distinction FR-012 is about: this must not read as somebody else's edit.
    expect(String(words).toLowerCase()).toMatch(/reach|connect/);
    expect(String(words).toLowerCase()).not.toMatch(/someone else/);
    expect(await editorText()).toContain('offline');
    expect(onDisk('notes.md')).toBe('original\n');
  });

  it('writes nothing on its own while autosave is off', async () => {
    // FR-007b. Off is the state a profile starts in, and the failure this guards is a default
    // that writes the developer's files for them before they have said it may.
    const before = (await requestsIssued()).filter((r) => r.method === 'write').length;
    await typeInEditor('unsaved ');
    await browser.pause(3000); // Longer than the debounce, so a timer that fired would show.
    expect((await requestsIssued()).filter((r) => r.method === 'write').length).toBe(before);
    expect(onDisk('notes.md')).toBe('original\n');
  });
});
