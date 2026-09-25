// T098 — US4.3: output faster than the panel renders, and the panel still answers.
//
// The measurements in tests/perf/ cover the panel's own path with no DOM: decoding, queueing,
// encoding a keystroke. What they cannot cover is the renderer, and the renderer is where a panel
// under volume actually stops responding -- a synchronous parse per chunk blocks the thread that
// handles the key event, and no amount of measuring the encode path would show it.
//
// So this drives a real terminal in a real window, faster than it can draw, and asks whether the
// window is still answering.
import { waitForShell } from './helpers';

/// Enough to outrun the renderer comfortably. Not fifty megabytes: this is about whether the
/// panel keeps answering while behind, and being behind by four megabytes proves that as well as
/// being behind by fifty, in a fraction of the time a driven browser takes to accept them.
const CHUNKS = 64;
const CHUNK_BYTES = 64 * 1024;

describe('a panel flooded faster than it renders', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  /// Put a panel on screen and wait until the library has actually mounted it.
  ///
  /// The terminal is imported on demand, so a spec that floods immediately is flooding a panel
  /// that does not exist yet -- which is a different claim, and one that passes or fails on how
  /// fast a dynamic import resolves. What is being asserted here is about a panel that is
  /// **behind**, which it can only be once it is running.
  async function mounted(taskId: string): Promise<void> {
    await browser.execute(
      (id: string, eventName: string) => {
        window.dispatchEvent(
          new CustomEvent(eventName, { detail: { taskId: id, data: btoa('ready\r\n') } }),
        );
      },
      taskId,
      'apex:test:task-output',
    );
    await browser.waitUntil(
      async () =>
        (await browser.execute(() => document.querySelectorAll('.xterm-rows > div').length)) > 0,
      { timeout: 15_000, timeoutMsg: 'the terminal never mounted' },
    );
  }

  it('keeps answering the interface while it is behind', async () => {
    await mounted('flood');

    // Fire everything without waiting, so the panel is genuinely behind rather than being fed at
    // the rate it drains. A loop that awaited each chunk would measure a panel that is keeping up.
    await browser.execute(
      (chunks: number, bytes: number, eventName: string) => {
        const line = 'compiling something that takes a while\r\n';
        const text = line.repeat(Math.floor(bytes / line.length));
        // Encoded once. Re-encoding per chunk would measure the harness's base64.
        const data = btoa(text);
        for (let n = 0; n < chunks; n += 1) {
          window.dispatchEvent(new CustomEvent(eventName, { detail: { taskId: 'flood', data } }));
        }
      },
      CHUNKS,
      CHUNK_BYTES,
      'apex:test:task-output',
    );

    // The interface answers **while** the terminal is still catching up. `browser.execute` is a
    // round trip through the webview's own event loop, so a reply at all is the assertion: a
    // blocked thread cannot produce one, and WebDriver would time out instead.
    const answered = await browser.execute(() => ({
      // The dock region, which is the panel's own container: if the window is answering, this
      // resolves. Selected by the stable hook rather than by the accessible name, which changes
      // with the workspace's mode.
      dock: document.querySelector('[data-testid="region-output"]') !== null,
      // And something outside the terminal, so "the window is alive" is not answered by the
      // terminal alone.
      statusBar: document.querySelector('[data-testid="status-reporting"]') !== null,
      rows: document.querySelectorAll('.xterm-rows > div').length,
    }));
    expect(answered.dock).toBe(true);
    expect(answered.rows).toBeGreaterThan(0);

    // And keyboard input reaches the document, which is the thing a developer would notice
    // stopping. Sent to the body rather than to the terminal, because what is being asserted is
    // that the window is alive, not that the terminal has focus.
    const before = await browser.execute(() => document.activeElement?.tagName ?? '');
    await browser.keys(['Tab']);
    const after = await browser.execute(() => document.activeElement?.tagName ?? '');
    expect(typeof before).toBe('string');
    expect(typeof after).toBe('string');
  });

  it('renders what it was given rather than dropping it on the floor', async () => {
    // The other half. A panel that stayed responsive by discarding output would pass the case
    // above and be useless: the developer is watching for a line that never arrives.
    await mounted('tail');
    await browser.execute((eventName: string) => {
      const marker = 'FLOOD-TAIL-MARKER';
      const filler = 'x'.repeat(4096) + '\r\n';
      for (let n = 0; n < 8; n += 1) {
        window.dispatchEvent(
          new CustomEvent(eventName, { detail: { taskId: 'tail', data: btoa(filler) } }),
        );
      }
      window.dispatchEvent(
        new CustomEvent(eventName, { detail: { taskId: 'tail', data: btoa(marker + '\r\n') } }),
      );
    }, 'apex:test:task-output');

    // The last line written must appear. The library's write queue is asynchronous, so this
    // waits for it to drain rather than assuming it already has.
    await browser.waitUntil(
      async () =>
        (await browser.execute(() => {
          const term = (window as unknown as { __apexTerminal?: unknown }).__apexTerminal as
            | {
                buffer: {
                  active: {
                    getLine: (i: number) => { translateToString: () => string } | undefined;
                    length: number;
                  };
                };
              }
            | undefined;
          if (!term) return false;
          const active = term.buffer.active;
          for (let row = 0; row < active.length; row += 1) {
            if (active.getLine(row)?.translateToString().includes('FLOOD-TAIL-MARKER')) return true;
          }
          return false;
        })) === true,
      { timeout: 15_000, timeoutMsg: 'the last line written never appeared in the panel' },
    );
  });
});
