// T013 — SC-001 and SC-002, and the feature's central claim.
//
// §1.4's one absolute rule: a keystroke renders with **zero** milliseconds of network in the
// path. Written as a count rather than a duration, because a count cannot be met by a fast
// network and can only be met by there being no request at all.
//
// The count comes from the sink, at the point a request actually leaves. A counter any further
// up would count intentions.
import {
  resetWorkspace,
  openWorkspace,
  openFile,
  editorText,
  requestsIssued,
  typeInEditor,
} from './editor-harness';
import { waitForShell, resetSession } from './helpers';

/// A hundred characters with no repeats adjacent and nothing a tokenizer treats specially.
const HUNDRED = Array.from({ length: 100 }, (_, i) => String.fromCharCode(97 + (i % 26))).join('');

describe('typing reaches no network', () => {
  before(async () => {
    resetWorkspace({ 'notes.md': 'start\n' });
    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
  });

  it('renders every character of a burst and issues nothing while doing it', async () => {
    await openFile('/notes.md');
    await browser.waitUntil(async () => (await editorText()).includes('start'), {
      timeout: 20_000,
      timeoutMsg: 'the file never arrived',
    });

    // Everything the open cost. Whatever happens after this point is what typing cost.
    const before = (await requestsIssued()).length;

    await typeInEditor(HUNDRED);

    const text = await editorText();
    for (const ch of new Set(HUNDRED.split(''))) {
      expect(text).toContain(ch);
    }
    expect(text.replace(/[^a-z]/g, '').length).toBeGreaterThanOrEqual(100);

    // The assertion the feature is built around. Not "few", not "fast": none.
    expect((await requestsIssued()).length - before).toBe(0);
  });

  it('issues nothing when the same file is opened again', async () => {
    // SC-002 the other way round. A reopen that refetched would also discard unsaved work
    // (FR-023), so this is two requirements standing on one observation.
    const before = (await requestsIssued()).length;
    await openFile('/notes.md');
    expect((await requestsIssued()).length - before).toBe(0);
  });
});
