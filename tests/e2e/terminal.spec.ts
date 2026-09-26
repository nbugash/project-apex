// T071 and T123 — the journey, through the real interface.
//
// # What this covers, and what it does not
//
// The developer's journey is: run a command, watch it, type into it, resize the panel, see it
// end -- and, after a connection drops, come back to find what was missed filled in ahead of
// anything since. This drives all of that against the real terminal in the real window.
//
// **The engine is not in the loop, and that is stated rather than implied.** The client's remote
// path is not composed end to end for any feature: `RemoteWorkspaceProvider` is built, tested and
// never constructed, and no concrete `RequestSender` exists in `client/core`. So output arrives
// through the harness seam and the panel's outbound calls are read back from the sink instead of
// from a socket.
//
// Per A-E2ESCOPE, each scenario says which level covers the half this cannot reach:
//
// - **Output rendering** is covered here, end to end, and there is no lower substitute: whether
//   bytes become the right cells is a claim about a renderer in a window.
// - **Input reaching the task** is covered at the engine level by `engine/tests/task_input.rs`,
//   which writes control bytes and invalid UTF-8 to a real process and compares what comes back
//   as bytes. What is checked here is the panel's half: that a keystroke leaves as the right
//   bytes, which is the part a renderer can get wrong.
// - **A resize reaching the process** is `engine/tests/task_resize.rs`, measured against
//   `fixture_winsize` asking the kernel. Here: that resizing the panel sends one at all.
// - **An ending** is `engine/tests/task_lifecycle.rs`, which asserts which field is present.
//   Here: that the panel renders what it is given.
// - **The replay's order** is `engine/tests/task_reattach.rs`, asserted as a sequence on the
//   wire. Here: that the panel shows it in that order once it arrives.
import { waitForShell } from './helpers';

/// What the panel sent, recorded by the sink under automation.
interface Sent {
  method: string;
  taskId: string;
  detail: string;
}

async function sent(): Promise<Sent[]> {
  return browser.execute(
    () => (window as unknown as { __apexSent?: Sent[] }).__apexSent ?? [],
  ) as Promise<Sent[]>;
}

async function clearSent(): Promise<void> {
  await browser.execute(() => {
    (window as unknown as { __apexSent?: unknown[] }).__apexSent = [];
  });
}

/// Deliver output to a task's panel, as `execution/onStdout` would.
async function output(taskId: string, text: string): Promise<void> {
  await browser.execute(
    (id: string, body: string, eventName: string) => {
      window.dispatchEvent(
        new CustomEvent(eventName, { detail: { taskId: id, data: btoa(body) } }),
      );
    },
    taskId,
    text,
    'apex:test:task-output',
  );
}

/// Everything the terminal is showing, row by row.
async function screen(): Promise<string> {
  return browser.execute(() => {
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
    if (!term) return '';
    const active = term.buffer.active;
    const rows: string[] = [];
    for (let row = 0; row < active.length; row += 1) {
      rows.push(active.getLine(row)?.translateToString() ?? '');
    }
    return rows.join('\n');
  });
}

async function mounted(taskId: string): Promise<void> {
  await output(taskId, 'ready\r\n');
  await browser.waitUntil(
    async () =>
      (await browser.execute(() => document.querySelectorAll('.xterm-rows > div').length)) > 0,
    { timeout: 15_000, timeoutMsg: 'the terminal never mounted' },
  );
}

describe('running a command and watching it through', () => {
  beforeEach(async () => {
    await waitForShell();
    await clearSent();
  });

  it('shows the output a task produces', async () => {
    await mounted('journey');
    await output('journey', '   Compiling apex-engine v0.1.0\r\n');
    await browser.waitUntil(async () => (await screen()).includes('Compiling apex-engine'), {
      timeout: 10_000,
      timeoutMsg: 'the output never reached the panel',
    });
  });

  it('sends what the developer types, as the bytes they typed', async () => {
    // The panel's half of SC-007. Whether those bytes reach the task unchanged is
    // `engine/tests/task_input.rs`, against a real process; what a renderer can get wrong is the
    // encoding on the way out, and that is what this reads back.
    await mounted('journey');
    await clearSent();

    await browser.execute(() => {
      const term = (window as unknown as { __apexTerminal?: unknown }).__apexTerminal as
        { input: (data: string) => void } | undefined;
      // `input` is xterm's own "as if the user typed this", so this goes through the same
      // `onData` a keypress does rather than around it.
      term?.input('x');
    });

    await browser.waitUntil(async () => (await sent()).some((s) => s.method === 'writeStdin'), {
      timeout: 10_000,
      timeoutMsg: 'the keystroke never left the panel',
    });
    const writes = (await sent()).filter((s) => s.method === 'writeStdin');
    // `x` is 0x78, which is `eA==`. Asserted as the encoded form because that is what goes on
    // the wire -- and because it is what a double encode would get wrong.
    expect(writes[0]?.detail).toBe('eA==');
  });

  it('tells the task its size when the panel is fitted', async () => {
    // Attaching deliberately sets no size (attach guarantee 9), so somebody has to. Whether the
    // size reaches the process is `engine/tests/task_resize.rs`, measured against the kernel.
    await clearSent();
    await mounted('sized');
    await browser.waitUntil(async () => (await sent()).some((s) => s.method === 'resize'), {
      timeout: 10_000,
      timeoutMsg: 'the panel never reported its size',
    });
    const resizes = (await sent()).filter((s) => s.method === 'resize');
    expect(resizes[0]?.detail).toMatch(/^\d+x\d+$/);
  });

  it('shows an ending rather than simply stopping', async () => {
    // The engine decides which field an ending carries (`engine/tests/task_lifecycle.rs`); the
    // panel's job is to show it, and a build that just stops leaves the developer waiting.
    await mounted('ending');
    await output('ending', '\r\nBUILD FAILED: exited 101\r\n');
    await browser.waitUntil(async () => (await screen()).includes('exited 101'), {
      timeout: 10_000,
      timeoutMsg: 'the ending never appeared',
    });
  });
});

describe('coming back to a build that kept going', () => {
  beforeEach(async () => {
    await waitForShell();
    await clearSent();
  });

  it('fills in what was missed before anything produced since', async () => {
    // T123. The order is the claim: BEFORE was seen, MISSED arrived while the client was away,
    // AFTER once it was back. The wire's ordering is `engine/tests/task_reattach.rs`; this is
    // the panel showing it in the order it arrived.
    await mounted('reconnect');
    await output('reconnect', 'BEFORE-THE-DROP\r\n');
    await browser.waitUntil(async () => (await screen()).includes('BEFORE-THE-DROP'), {
      timeout: 10_000,
      timeoutMsg: 'the first line never arrived',
    });

    // The replay, then what came after, in that order.
    await output('reconnect', 'MISSED-WHILE-AWAY\r\n');
    await output('reconnect', 'AFTER-THE-RETURN\r\n');

    await browser.waitUntil(async () => (await screen()).includes('AFTER-THE-RETURN'), {
      timeout: 10_000,
      timeoutMsg: 'the panel never carried on',
    });

    const shown = await screen();
    const before = shown.indexOf('BEFORE-THE-DROP');
    const missed = shown.indexOf('MISSED-WHILE-AWAY');
    const after = shown.indexOf('AFTER-THE-RETURN');
    expect(before).toBeGreaterThanOrEqual(0);
    expect(missed).toBeGreaterThan(before);
    expect(after).toBeGreaterThan(missed);
  });

  it('tells the developer what survived and how much was missed', async () => {
    // FR-032, in the panel they are looking at. The count comes from attach's `retained`, never
    // from a per-frame marker: a replayed frame is deliberately indistinguishable from a live
    // one, so a flag would exist only to be ignored or to draw a seam that is not in the output.
    await mounted('summary');
    await browser.execute((eventName: string) => {
      window.dispatchEvent(
        new CustomEvent(eventName, {
          detail: {
            outcomes: [
              { kind: 'survived', taskId: 'summary', retained: 4096 },
              { kind: 'finished', taskId: 'other', ending: 'exited 0' },
            ],
            retained: 4096,
          },
        }),
      );
    }, 'apex:test:reconnected');

    await browser.waitUntil(async () => (await screen()).includes('[reconnected]'), {
      timeout: 10_000,
      timeoutMsg: 'the summary never appeared in the panel',
    });
    const shown = await screen();
    expect(shown).toContain('1 still running');
    expect(shown).toContain('other exited 0');
    expect(shown).toContain('4 KB missed');
  });

  it('carries on rendering after the summary', async () => {
    // A summary that ended the transcript would be worse than none: the build is still going.
    await mounted('carryon');
    await browser.execute((eventName: string) => {
      window.dispatchEvent(
        new CustomEvent(eventName, {
          detail: {
            outcomes: [{ kind: 'survived', taskId: 'carryon', retained: 0 }],
            retained: 0,
          },
        }),
      );
    }, 'apex:test:reconnected');
    await output('carryon', 'STILL-COMPILING\r\n');

    await browser.waitUntil(async () => (await screen()).includes('STILL-COMPILING'), {
      timeout: 10_000,
      timeoutMsg: 'the panel stopped rendering after the summary',
    });
  });
});
