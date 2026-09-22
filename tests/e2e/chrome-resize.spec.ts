// T022 — US1. Fixed-width surfaces keep their widths as the window resizes (FR-010).
import { waitForShell, token } from './helpers';

const widthOf = async (selector: string): Promise<number> => (await $(selector).getSize()).width;

describe('chrome under resize', () => {
  before(async () => {
    await waitForShell();
  });

  after(async () => {
    // Leave the window as the rest of the suite expects to find it.
    await browser.setWindowSize(1200, 800);
  });

  it('keeps the rail and tool window fixed while the document area absorbs the change', async () => {
    await browser.setWindowSize(1400, 900);
    await browser.pause(200);

    const railAtWide = await widthOf('nav.rail');
    const toolAtWide = await widthOf('aside.tool-window');
    const mainAtWide = await widthOf('main.document-area');

    await browser.setWindowSize(1000, 700);
    await browser.pause(200);

    expect(await widthOf('nav.rail')).toBe(railAtWide);
    expect(await widthOf('aside.tool-window')).toBe(toolAtWide);

    // The flexible region is the one that gives. Asserting it shrank — rather than only
    // that the fixed ones held — catches the case where the whole layout overflows the
    // window instead of reflowing, which leaves the fixed widths correct and the
    // application unusable.
    expect(await widthOf('main.document-area')).toBeLessThan(mainAtWide);
  });

  it('matches the tokens at both sizes', async () => {
    const railToken = Number.parseFloat(await token('--vk-rail-width'));
    const toolToken = Number.parseFloat(await token('--vk-tool'));

    for (const [w, h] of [
      [900, 650],
      [1600, 1000],
    ]) {
      await browser.setWindowSize(w!, h!);
      await browser.pause(200);
      expect(await widthOf('nav.rail')).toBe(railToken);
      expect(await widthOf('aside.tool-window')).toBe(toolToken);
    }
  });

  it('never lets the chrome header change height', async () => {
    const headerToken = Number.parseFloat(await token('--vk-chrome-height'));
    await browser.setWindowSize(800, 600);
    await browser.pause(200);
    expect((await $('header.chrome').getSize()).height).toBe(headerToken);
  });
});
