// T033 — US2. Tool window state survives a restart (FR-011, SC-005).
import { waitForShell, resetSession, relaunch, readSession, waitForPersisted } from './helpers';

const toolWindow = () =>
  readSession()?.tool_window as
    { active_destination_id: string | null; collapsed: boolean; width: number } | undefined;

describe('tool window state across restart', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('restores a collapsed panel as collapsed', async () => {
    await $('nav.rail [role="tab"][aria-selected="true"]').click();
    await waitForPersisted((s) => (s.tool_window as { collapsed: boolean }).collapsed, 'collapse');

    await relaunch();
    expect(await $('aside.tool-window').isExisting()).toBe(false);
    expect(toolWindow()?.collapsed).toBe(true);

    // And the rail still marks the destination, as the prototype does while collapsed —
    // otherwise a restart would lose the only cue to what reopening will show.
    expect((await $$('nav.rail [role="tab"][aria-selected="true"]')).length).toBe(1);
  });

  it('restores an expanded panel at its persisted width', async () => {
    await $('nav.rail [role="tab"][aria-selected="true"]').click();
    await $('aside.tool-window').waitForExist({ timeout: 5000 });

    const splitter = await $('[aria-label="Resize tool window"]');
    await splitter.click();
    for (let i = 0; i < 5; i++) await browser.keys('ArrowRight');

    const widened = (await $('aside.tool-window').getSize()).width;
    await waitForPersisted((s) => (s.tool_window as { width: number }).width === widened, 'width');

    await relaunch();
    expect((await $('aside.tool-window').getSize()).width).toBe(widened);
  });

  it('keeps the active destination across a restart', async () => {
    const before = toolWindow()?.active_destination_id;
    expect(before).not.toBe(null);

    await relaunch();
    expect(toolWindow()?.active_destination_id).toBe(before);
  });
});
