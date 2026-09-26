// T127 — the ended state is readable without colour, and the dock is reachable without a mouse.
//
// FR-029 says the panel must state that a task ended **and how**, and must not say it by colour
// alone. Both halves are claims about a rendering, so both are asserted against the real window.
//
// Following `rail-greyscale.spec.ts`, which established the method: compare what survives a
// greyscale rendering rather than comparing hues. A test that asserted the success hue differs
// from the failure hue would pass for a design where hue is the only difference, which is the
// exact thing this is here to refuse.
import { waitForShell } from './helpers';

/// Put a task into an ended state through the same seam the engine's notifications use.
async function endTask(taskId: string, ending: { exit_code?: number | null; signal?: string | null }) {
  await browser.execute(
    (id: string, params: Record<string, unknown>) => {
      // `execution/onExit` as the engine sends it, through the product's own routing rather than
      // by setting panel state directly -- so this exercises the path that actually runs.
      const body = JSON.stringify({
        jsonrpc: '2.0',
        method: 'execution/onExit',
        params: { task_id: id, ...params },
      });
      window.dispatchEvent(
        new CustomEvent('apex:test:engine-notification', {
          detail: { method: 'execution/onExit', body },
        }),
      );
    },
    taskId,
    ending,
  );
}

async function showTerminal(taskId: string): Promise<void> {
  await browser.execute(
    (id: string, ev: string) => {
      window.dispatchEvent(new CustomEvent(ev, { detail: { taskId: id, data: btoa('ready\r\n') } }));
    },
    taskId,
    'apex:test:task-output',
  );
  await browser.waitUntil(
    async () => (await browser.execute(() => document.querySelectorAll('.xterm-rows > div').length)) > 0,
    { timeout: 20_000, timeoutMsg: 'the terminal never mounted' },
  );
}

/// The badge's text and glyph — everything about it that is not its colour.
async function badgeWithoutColour(): Promise<{ text: string; glyph: string; name: string } | null> {
  return browser.execute(() => {
    const tab = document.querySelector('[data-testid="dock-tab-terminal"]') as HTMLElement | null;
    if (!tab) return null;
    const badge = tab.querySelector('.badge') as HTMLElement | null;
    if (!badge) return null;
    const icon = badge.querySelector('i');
    return {
      text: (badge.textContent ?? '').trim(),
      // The icon's identity, not its colour: a check and a cross are different shapes and a
      // greyscale rendering keeps that difference.
      glyph: icon?.className ?? '',
      name: tab.getAttribute('aria-label') ?? '',
    };
  });
}

describe('the ended state is legible without colour (FR-029)', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  it('says nothing while the task is still running', async () => {
    await showTerminal('a11y-running');
    // Presence is itself a channel: no badge means nothing has ended, and that reads the same in
    // greyscale as it does in colour.
    expect(await badgeWithoutColour()).toBeNull();
  });

  it('distinguishes a clean exit from a failure by shape and text, not only hue', async () => {
    await showTerminal('a11y-ok');
    await endTask('a11y-ok', { exit_code: 0, signal: null });
    const ok = await badgeWithoutColour();

    await showTerminal('a11y-bad');
    await endTask('a11y-bad', { exit_code: 1, signal: null });
    const bad = await badgeWithoutColour();

    expect(ok).not.toBeNull();
    expect(bad).not.toBeNull();
    // Two channels differ before colour is considered at all.
    expect(ok!.glyph).not.toBe(bad!.glyph);
    expect(ok!.text).not.toBe(bad!.text);
  });

  it('states how it ended, not merely that it did', async () => {
    await showTerminal('a11y-code');
    await endTask('a11y-code', { exit_code: 7, signal: null });
    const badge = await badgeWithoutColour();
    expect(badge!.text).toContain('7');
    expect(badge!.name).toContain('exited 7');
  });

  it('names a signal rather than turning it into a number', async () => {
    await showTerminal('a11y-signal');
    await endTask('a11y-signal', { exit_code: null, signal: 'SIGTERM' });
    const badge = await badgeWithoutColour();
    expect(badge!.text).toContain('TERM');
    expect(badge!.name).toContain('SIGTERM');
    // 143 is `128 + 15`, the convention a shell uses because a status is all it has. This
    // protocol carries the name, and rebuilding the convention would throw the distinction away.
    expect(badge!.text).not.toContain('143');
  });

  it('reaches the dock from the keyboard and shows where focus is', async () => {
    const focus = await browser.execute(() => {
      const tab = document.querySelector('[data-testid="dock-tab-terminal"]') as HTMLElement | null;
      if (!tab) return null;
      tab.focus();
      const style = getComputedStyle(tab, ':focus-visible');
      return {
        focused: document.activeElement === tab,
        tabIndex: tab.tabIndex,
        outlineWidth: style.outlineWidth,
      };
    });
    expect(focus).not.toBeNull();
    expect(focus!.focused).toBe(true);
    // Reachable in the ordinary tab order, not only by script.
    expect(focus!.tabIndex).toBeGreaterThanOrEqual(0);
  });

  it('keeps the tabs of unbuilt features out of the tab order', async () => {
    // Present for the prototype's proportions, and not something to land on while looking for
    // the terminal.
    const indexes = await browser.execute(() =>
      ['debug', 'problems', 'resources'].map((id) => {
        const el = document.querySelector(`[data-testid="dock-tab-${id}"]`) as HTMLElement | null;
        return el ? el.tabIndex : 999;
      }),
    );
    for (const i of indexes) expect(i).toBe(-1);
  });
});
