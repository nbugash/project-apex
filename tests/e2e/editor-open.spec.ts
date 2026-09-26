// T020 — US1's acceptance scenarios, against a real engine and a real directory.
//
// The claims here are the ones no unit test reaches: that a file the developer clicks actually
// opens, that a binary one is declined rather than shown as mojibake, and that syntax colour
// survives a dropped connection because it was never on the far side of it.
import {
  resetWorkspace,
  openWorkspace,
  openFile,
  editorText,
  requestsIssued,
  typeInEditor,
} from './editor-harness';
import { waitForShell, resetSession } from './helpers';

describe('opening a file', () => {
  before(async () => {
    resetWorkspace({
      'src/main.rs': 'fn main() {\n    println!("hello");\n}\n',
      'notes.md': '# Notes\n',
      // A PNG header: bytes that are legal file content and are not text. Written as real
      // bytes rather than as a string with odd characters in it, because `from_utf8` on a
      // string that happens to encode is not the case under test.
      'logo.png': Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0xfe, 0x00]),
    });
    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
  });

  it('fetches an uncached file exactly once', async () => {
    // SC-002. One open is one request: a provider call that fanned out would make every count
    // in this feature meaningless, and nothing else here would notice.
    await openFile('/src/main.rs');
    await browser.waitUntil(async () => (await editorText()).includes('fn main'), {
      timeout: 20_000,
      timeoutMsg: 'the file never arrived',
    });

    const reads = (await requestsIssued()).filter(
      (r) => r.path === '/src/main.rs' && r.method.startsWith('read'),
    );
    expect(reads).toHaveLength(1);
    expect(await editorText()).toContain('println!("hello")');
  });

  it('declines a file that is not text, and says what will show it', async () => {
    await openFile('/logo.png');
    const notice = await $('[data-testid="editor-notice"]');
    await notice.waitForDisplayed({ timeout: 20_000 });
    expect(await notice.getAttribute('data-kind')).toBe('binary');
    // Mojibake would be worse than a refusal, and an empty editor worse still: it looks like a
    // file that is there and has nothing in it.
    expect(await editorText()).toBe('');
  });

  it('keeps the buffer and the syntax colour when the connection drops', async () => {
    // FR-003 and FR-005. Colour is local by design (§8.1), and a requirement satisfied only by
    // accident is one a later change removes without anybody noticing — so it is asserted on
    // the rendered tokens rather than on the fact that Monaco was configured.
    await openFile('/src/main.rs');
    await browser.waitUntil(async () => (await editorText()).includes('fn main'), {
      timeout: 20_000,
    });

    const coloured = async (): Promise<number> =>
      (await browser.execute(() => {
        const spans = document.querySelectorAll('.view-lines span[class*="mtk"]');
        const seen = new Set<string>();
        spans.forEach((s) => seen.add(getComputedStyle(s).color));
        return seen.size;
      })) as number;

    const before = await coloured();
    expect(before).toBeGreaterThan(1);

    await browser.execute(() => {
      const fn = (
        window as unknown as {
          __TAURI_INTERNALS__?: { invoke: (c: string, a: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__?.invoke;
      void fn?.('stub_set_connection', { state: 'disconnected' });
    });

    // Typing continues, and every character arrives, because nothing in the path waits on a
    // network (SC-009, §1.4's absolute rule).
    await typeInEditor('// offline edit');

    expect(await editorText()).toContain('// offline edit');
    expect(await coloured()).toBeGreaterThan(1);
  });
});
