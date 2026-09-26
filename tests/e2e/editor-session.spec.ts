// T057 — US4's acceptance scenarios. What is open comes back, and so does what is in it.
//
// SC-008 is about the tabs; the rest is about there being something behind them. A tab
// restored as a label with nothing in it is what FR-022 forbids, and it is what this
// application did until tabs learned to remember which file they were of.
import {
  resetWorkspace,
  openWorkspace,
  openFile,
  editorText,
  removeOnHost,
} from './editor-harness';
import { waitForShell, resetSession, readSession } from './helpers';

async function tabPaths(): Promise<string[]> {
  const s = readSession();
  return ((s?.documents as Array<{ path: string }> | undefined) ?? []).map((d) => d.path);
}

describe('coming back to what was open', () => {
  before(async () => {
    resetWorkspace({
      'src/main.rs': 'fn main() {}\n',
      'notes.md': '# Notes\n',
      'doomed.txt': 'here for now\n',
    });
    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
    await openFile('/notes.md');
    await openFile('/src/main.rs');
    await openFile('/doomed.txt');
    await browser.waitUntil(async () => (await tabPaths()).length === 3, {
      timeout: 20_000,
      timeoutMsg: 'the tabs were never persisted',
    });
  });

  it('restores the same tabs in the same order with the same one focused', async () => {
    // SC-008.
    const before = await tabPaths();
    const focusedBefore = readSession()?.focused_document_id;

    await browser.reloadSession();
    await waitForShell();

    expect(await tabPaths()).toEqual(before);
    expect(readSession()?.focused_document_id).toBe(focusedBefore);
    expect(await $$('.strip [role="tab"]')).toHaveLength(3);
  });

  it('shows the focused tab its content again', async () => {
    // FR-021. The tab list alone is a row of labels; this is what gives it something behind it.
    await openWorkspace();
    await browser.waitUntil(async () => (await editorText()).includes('here for now'), {
      timeout: 30_000,
      timeoutMsg: 'the restored tab never showed its file',
    });
  });

  it('reports a restored tab whose file has gone rather than showing an empty document', async () => {
    // FR-022. An empty editor looks like a file that is there and happens to be empty, and
    // saving it would create one — which is how a restored tab truncates a file that somebody
    // moved between sessions.
    removeOnHost('doomed.txt');

    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();

    const notice = await $('[data-testid="editor-notice"]');
    await notice.waitForDisplayed({ timeout: 30_000 });
    expect(await notice.getAttribute('data-kind')).toBe('missing');
    expect(await editorText()).toBe('');
  });
});
