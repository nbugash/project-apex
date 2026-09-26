// A real terminal, end to end, with nothing simulated.
//
// Every other spec in this suite drives the panel through `harness.ts`, which writes bytes in
// directly because until now there was no way to start a task from the interface. This one
// starts a real process on a real engine and types into it, which is the claim the feature
// actually makes and the one no combination of the others adds up to: both halves were correct
// and unjoined, and each half's tests passed throughout.
//
// The engine here is a child process rather than one on an instance (`LocalEngineSpawner`). It
// is the same binary speaking the same protocol over the same transport, with a pipe where the
// network would be -- so what this proves about the joined path holds for the remote one, minus
// the network itself, which F001's own suite covers against its mock.
import { waitForShell } from './helpers';

const ROOT = process.cwd();

/// Everything the terminal is showing, row by row.
async function screen(): Promise<string> {
  return browser.execute(() => {
    const t = (
      window as unknown as {
        __apexTerminal?: {
          buffer: {
            active: {
              getLine: (i: number) => { translateToString: () => string } | undefined;
              length: number;
            };
          };
        };
      }
    ).__apexTerminal;
    if (!t) return '';
    const a = t.buffer.active;
    const rows: string[] = [];
    for (let i = 0; i < a.length; i += 1) rows.push(a.getLine(i)?.translateToString() ?? '');
    return rows.join('\n');
  });
}

async function openWorkspace(): Promise<void> {
  await browser.execute(
    async (root: string) => {
      const fn = (
        window as unknown as {
          __TAURI_INTERNALS__?: { invoke: (c: string, a: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__?.invoke;
      // The engine will not accept a task for a workspace it has not registered, and this is
      // the call that tells it. Awaited, so the terminal below cannot race it.
      await fn?.('workspace_open', { name: 'apex', host: 'localhost', basePath: root });
    },
    ROOT,
  );
}

describe('a live terminal', () => {
  it('runs a real process and answers what is typed into it', async () => {
    await waitForShell();
    await openWorkspace();

    // The click is the whole point: nothing has started a shell before this, and nothing
    // should have. Clicking the tab is how a person starts one, in this application and in the
    // two it is imitating.
    const tab = await $('[data-testid="dock-tab-terminal"]');
    await tab.waitForDisplayed({ timeout: 15_000 });
    await tab.click();

    await browser.waitUntil(
      async () =>
        (await browser.execute(() => document.querySelectorAll('.xterm-rows > div').length)) > 0,
      { timeout: 30_000, timeoutMsg: 'the terminal never mounted' },
    );

    // A command whose answer this test did not write, so what is asserted is the process's
    // output and not an echo of the request. `paste` rather than per-key events because what is
    // under test is the round trip, and `terminal.spec.ts` already covers keystroke encoding.
    await browser.execute(() => {
      const t = (window as unknown as { __apexTerminal?: { paste: (s: string) => void } })
        .__apexTerminal;
      t?.paste('echo apex-live-terminal-$((6*7))\r');
    });

    // The arithmetic is the point: a shell evaluated it. An engine echoing input back would
    // show the literal `$((6*7))`, and so would a harness replaying what the panel sent.
    await browser.waitUntil(async () => (await screen()).includes('apex-live-terminal-42'), {
      timeout: 30_000,
      timeoutMsg: 'the shell never answered',
    });

    expect(await screen()).toContain('apex-live-terminal-42');
  });

  it('starts one shell on the first click and reuses it afterwards', async () => {
    // What VS Code and IntelliJ do, and the reason this is a tab rather than a button: the
    // panel is a place you go back to, and going back to it must not leave a process behind
    // each time. A login shell runs the developer's whole profile, so a second one is not a
    // harmless duplicate.
    const runs = async () =>
      (
        (await browser.execute(
          () => (window as unknown as { __apexSent?: Array<{ method: string }> }).__apexSent ?? [],
        )) as Array<{ method: string }>
      ).filter((s) => s.method === 'run').length;

    // One start from the first test's click, and nothing since.
    expect(await runs()).toBe(1);

    const tab = await $('[data-testid="dock-tab-terminal"]');
    // Closes the dock, as clicking the current tab does in both editors.
    await tab.click();
    await browser.pause(300);
    // And opens it again on the terminal already running.
    await tab.click();
    await browser.pause(800);

    expect(await runs()).toBe(1);
    // The scrollback survived, which is what makes it the same terminal rather than a new one
    // that happens to look alike.
    expect(await screen()).toContain('apex-live-terminal-42');
    // And it is **drawn**, not merely held. `buffer.active` is the model, and reading only the
    // model passes for a terminal that moved into the new element without repainting -- which
    // is exactly what happened the first time this test was written.
    const painted = await browser.execute(
      () => document.querySelector('.xterm-rows')?.textContent ?? '',
    );
    expect(painted).toContain('apex-live-terminal-42');
  });
});
