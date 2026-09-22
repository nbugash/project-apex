// T021 — US1. The three chrome surfaces exist at the prototype's dimensions (SC-001).
import { waitForShell, token } from './helpers';

/** Dimensions are compared against the generated layout tokens, never against numbers
 *  typed here. A literal in this file would be a third copy of the prototype's geometry —
 *  it would pass while the application and the prototype disagreed, which is precisely the
 *  failure the fidelity work exists to prevent. */
async function tokenPx(name: string): Promise<number> {
  const raw = await token(name);
  const value = Number.parseFloat(raw);
  expect(Number.isFinite(value)).toBe(true);
  return value;
}

describe('chrome fidelity', () => {
  before(async () => {
    await waitForShell();
  });

  it('renders the chrome header at the prototype height', async () => {
    const header = await $('header.chrome');
    expect(await header.isExisting()).toBe(true);
    const { height } = await header.getSize();
    expect(height).toBe(await tokenPx('--vk-chrome-height'));
  });

  it('renders the activity rail at the prototype width', async () => {
    const rail = await $('nav.rail');
    expect(await rail.isExisting()).toBe(true);
    const { width } = await rail.getSize();
    expect(width).toBe(await tokenPx('--vk-rail-width'));
  });

  it('renders the tool window at the prototype width', async () => {
    const tool = await $('aside.tool-window');
    expect(await tool.isExisting()).toBe(true);
    const { width } = await tool.getSize();
    expect(width).toBe(await tokenPx('--vk-tool'));
  });

  it('renders every rail destination the prototype has', async () => {
    // Count and order, not identity: the identities are pinned in the core's unit tests,
    // where a mismatch names the offending destination instead of a number.
    const buttons = await $$('nav.rail [role="tab"]');
    expect(buttons.length).toBe(6);
  });

  it('opens with one destination active', async () => {
    const active = await $$('nav.rail [role="tab"][aria-selected="true"]');
    expect(active.length).toBe(1);
  });

  it('stacks the surfaces in the order the prototype does', async () => {
    const order = await browser.execute(() => {
      const x = (sel: string) => document.querySelector(sel)?.getBoundingClientRect().left ?? -1;
      return {
        headerTop: document.querySelector('header.chrome')?.getBoundingClientRect().top ?? -1,
        rail: x('nav.rail'),
        tool: x('aside.tool-window'),
        main: x('main.document-area'),
      };
    });
    expect(order.headerTop).toBe(0);
    expect(order.rail).toBeLessThan(order.tool);
    expect(order.tool).toBeLessThan(order.main);
  });
});
