// T047 — SC-012 and SC-013, which are each other's failure mode.
//
// A save changes the file, the host reports the file changed, and the developer must not be
// told somebody else edited it. Fix that by ignoring events for open files and SC-013 breaks;
// fix SC-013 by reporting every event and SC-012 breaks. A change made for one of them looks
// correct from whichever side you are standing on, so both are asserted here, together.
//
// **What is simulated and what is not.** The hash comparison — A-WRITEECHO, the thing that
// tells our own write from a colleague's — is real, and so is the file on disk. The *delivery*
// of the event is not: F004 built `file_event_notification.rs` and no caller, so nothing in the
// client forwards `workspace/onFileEvent` to the webview yet. The event here is dispatched the
// way the engine's will be, which is what makes the rule testable before that gap is closed.
import {
  resetWorkspace,
  openWorkspace,
  openFile,
  editorText,
  typeInEditor,
  clickSave,
  onDisk,
  editOnHost,
} from './editor-harness';
import { waitForShell, resetSession } from './helpers';

async function hostSays(path: string, event: string): Promise<void> {
  await browser.execute(
    (p: string, e: string) => {
      window.dispatchEvent(
        new CustomEvent('apex:test:file-event', {
          detail: { events: [{ relative_path: p, event: e }] },
        }),
      );
    },
    path,
    event,
  );
  await browser.pause(700);
}

async function noticeKind(): Promise<string | null> {
  return browser.execute(
    () => document.querySelector('[data-testid="editor-notice"]')?.getAttribute('data-kind') ?? null,
  );
}

describe('hearing about a change', () => {
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

  it('says nothing about a change that was our own save', async () => {
    // SC-012. Reporting the developer's own write back to them is the noise that makes every
    // later notice ignorable, and the notice this one is training them to ignore is the one
    // that says a colleague changed the file.
    await typeInEditor('ours ');
    await clickSave();
    await browser.waitUntil(async () => onDisk('notes.md').includes('ours'), { timeout: 20_000 });

    await hostSays('/notes.md', 'modified');

    expect(await noticeKind()).toBe(null);
    expect(await editorText()).toContain('ours');
  });

  it('reports a change that was not ours', async () => {
    // SC-013, and the reason the rule above cannot be "ignore events for open files".
    await typeInEditor('mine ');
    editOnHost('notes.md', 'theirs\n');

    await hostSays('/notes.md', 'modified');

    expect(await noticeKind()).toBe('diverged');
    // Told, not overwritten: the unsaved work is still there (FR-024b).
    expect(await editorText()).toContain('mine');
  });

  it('takes the host version when there is nothing to lose', async () => {
    editOnHost('notes.md', 'theirs\n');
    await hostSays('/notes.md', 'modified');

    await browser.waitUntil(async () => (await editorText()) === 'theirs\n', {
      timeout: 20_000,
      timeoutMsg: 'a clean buffer did not refresh',
    });
  });

  it('reports a deletion without throwing the buffer away', async () => {
    // FR-025. Discarding it would destroy unsaved work because somebody else removed the file.
    await typeInEditor('only here ');
    await hostSays('/notes.md', 'deleted');

    expect(await noticeKind()).toBe('missing');
    expect(await editorText()).toContain('only here');
  });
});
