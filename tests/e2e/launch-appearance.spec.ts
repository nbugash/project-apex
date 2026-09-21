// T058 — US4. No light or unstyled frame during launch (FR-020, SC-011).
//
// WebDriver attaches after the window exists, so it cannot capture frames from creation.
// What it CAN verify is the structural guarantee that makes the requirement hold: the
// window is configured hidden, and the first thing an attached session ever sees is the
// styled ground. If styles had not applied before the window was shown, the background
// here would be the platform default rather than the design system's ground.
import { token, waitForShell, relaunch, resetSession } from './helpers';

describe('launch appearance', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('presents the design system ground, never a platform default', async () => {
    const ground = await token('--color-bg');
    expect(ground).not.toBe('');

    const body = await browser.execute(() => getComputedStyle(document.body).backgroundColor);
    const shell = await browser.execute(
      () => getComputedStyle(document.querySelector('.shell')!).backgroundColor,
    );

    // Neither surface may be transparent or white, which is what an unstyled frame looks like.
    for (const colour of [body, shell]) {
      expect(colour).not.toBe('rgba(0, 0, 0, 0)');
      expect(colour).not.toBe('rgb(255, 255, 255)');
    }
  });

  it('has the design system stylesheet applied, not merely loaded', async () => {
    const applied = await browser.execute(() =>
      getComputedStyle(document.documentElement).getPropertyValue('--color-accent').trim(),
    );
    expect(applied).not.toBe('');
  });

  it('renders text in the design system font', async () => {
    const family = await browser.execute(
      () => getComputedStyle(document.querySelector('.shell')!).fontFamily,
    );
    expect(family.toLowerCase()).toContain('inter');
  });
});
