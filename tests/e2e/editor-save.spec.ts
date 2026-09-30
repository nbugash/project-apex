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
import { waitForShell, resetSession } from './helpers';
import { goOffline } from './offline-harness';

async function noticeTone(): Promise<string> {
  const notice = await $('[data-testid="editor-ending"]');
  await notice.waitForDisplayed({ timeout: 20_000 });
  return (await notice.getAttribute('data-tone')) ?? '';
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

  it('holds work on this machine when the engine cannot be reached, and keeps it', async () => {
    // **It ends the engine.** `stub_set_connection` cannot serve here:
    // with an engine present the composition root binds the real transport as the connection
    // source, so the stub drives something nothing is reading (see `wdio.conf.ts`). The honest
    // way to test "the engine could not be reached" is for it not to be reachable.
    //
    // **F012 changed what this asserts, deliberately.** Until F012 an unreachable engine meant
    // the save failed: tone `error`, wording about the link. Now the connection state has gone to
    // `Disconnected` by the time the save is made, so `file_write` holds the work locally and
    // reports `heldLocally` -- a success (FR-010, FR-011). What F006 protected still holds and is
    // still asserted: the save must not read as somebody else's edit (FR-012), the work stays in
    // the editor, and nothing reached the disk.
    //
    // **Held, not only ended.** The reconnection loop brings an ended engine back within about a
    // second, and the fixed pause this test used to take was long enough for it to: the save then
    // reached the host and read "saved". `goOffline` holds reconnection off for this process and
    // waits until the client has noticed, rather than guessing how long that takes. The next test
    // starts a fresh process, so the hold cannot leak into it.
    await typeInEditor('offline ');
    await goOffline();

    await clickSave();

    expect(await noticeTone()).toBe('ok');
    const words = await browser.execute(
      () => document.querySelector('[data-testid="editor-ending"]')?.textContent ?? '',
    );
    const said = String(words).toLowerCase();
    expect(said).toMatch(/this machine|locally/);
    expect(said).toMatch(/host|connection/);
    expect(said).not.toMatch(/someone else/);
    expect(said).not.toMatch(/not saved/);
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
