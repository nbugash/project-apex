// T054 — US3's acceptance scenarios. A file too big for one frame, opened without waiting.
//
// The sizes here are the plan's, not round numbers chosen for the test: 512 KiB is
// `wire::MAX_INLINE_READ`, which A-BULKSIZE fixed because content travels base64 and a
// threshold at §4.1's 1 MiB cap would encode past it.
import {
  resetWorkspace,
  openWorkspace,
  openFile,
  editorText,
  requestsIssued,
  responsesReceived,
} from './editor-harness';
import { waitForShell, resetSession } from './helpers';

/// §4.1's cap, which no single response may exceed.
const MAX_FRAME_BYTES = 1024 * 1024;
const THRESHOLD = 512 * 1024;

/// A file of numbered lines, so a position in it is identifiable rather than a run of one
/// character. `LINE` is 32 bytes, so the arithmetic below is exact.
const LINE = (i: number): string => `line ${String(i).padStart(10, '0')} of the big file\n`;
const BIG_LINES = 40_000; // ~1.4 MB: comfortably past the threshold, far short of the maximum.

describe('a file larger than one frame', () => {
  before(async () => {
    let big = '';
    for (let i = 0; i < BIG_LINES; i += 1) big += LINE(i);
    resetWorkspace({ 'big.log': big, 'small.txt': 'a small file\n' });
    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
  });

  it('shows the beginning without waiting for the whole file', async () => {
    await openFile('/big.log');
    await browser.waitUntil(async () => (await editorText()).includes('line 0000000000'), {
      timeout: 30_000,
      timeoutMsg: 'the first window never rendered',
    });

    const text = await editorText();
    expect(text.length).toBeGreaterThan(0);
    // The whole file is not held, which is the point: the window rendered before it could be.
    expect(new TextEncoder().encode(text).length).toBeLessThan(BIG_LINES * 32);
  });

  it('will not let a partly loaded file be edited', async () => {
    // A whole-file write of a partial buffer replaces the unloaded regions with nothing, and
    // §4.8 carries content rather than a patch, so there is no safe partial write.
    const partial = await $('[data-testid="editor-partial"]');
    await partial.waitForDisplayed({ timeout: 20_000 });
    const readOnly = await browser.execute(
      () =>
        (
          window as unknown as {
            __apexEditor?: { getOption: (id: number) => unknown; getRawOptions?: () => { readOnly?: boolean } };
          }
        ).__apexEditor?.getRawOptions?.()?.readOnly ?? null,
    );
    expect(readOnly).toBe(true);
  });

  it('fetches more when asked for the rest, and no response exceeds the frame limit', async () => {
    const before = (await requestsIssued()).filter((r) => r.method === 'readRange').length;

    await (await $('[data-testid="editor-load-rest"]')).click();
    await browser.waitUntil(
      async () => (await requestsIssued()).filter((r) => r.method === 'readRange').length > before,
      { timeout: 30_000, timeoutMsg: 'no further window was fetched' },
    );

    // The assertion §4.1 actually makes. Measured on what came back, because a request for a
    // legal amount can still be answered with more than fits if anything miscounts.
    for (const r of await responsesReceived()) {
      expect(r.bytes).toBeLessThanOrEqual(MAX_FRAME_BYTES);
    }
  });

  it('reads a file below the threshold whole, in one request', async () => {
    // FR-018. A range request for a small file costs the same round trip and delivers less.
    const before = (await requestsIssued()).length;
    await openFile('/small.txt');
    await browser.waitUntil(async () => (await editorText()).includes('a small file'), {
      timeout: 20_000,
      timeoutMsg: 'the small file never arrived',
    });

    const issued = (await requestsIssued()).slice(before);
    expect(issued.filter((r) => r.path === '/small.txt')).toHaveLength(1);
    expect(issued.filter((r) => r.path === '/small.txt')[0]!.method).toBe('read');

    const received = (await responsesReceived()).filter((r) => r.path === '/small.txt');
    expect(received).toHaveLength(1);
    expect(received[0]!.bytes).toBeLessThan(THRESHOLD);
  });
});
