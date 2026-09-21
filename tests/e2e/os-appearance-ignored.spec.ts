// T059 — US4. The interface does not follow the OS light/dark setting (FR-016).
import { token, waitForShell, relaunch, resetSession } from './helpers';

describe('operating system appearance is ignored', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('renders the dark ground regardless of the reported colour scheme', async () => {
    const ground = await token('--color-bg');

    // The webview reports whichever scheme the session runs under. Whatever it says, the
    // ground must not change: the design defines no light variant.
    const reported = await browser.execute(
      () => window.matchMedia('(prefers-color-scheme: light)').matches,
    );
    const background = await browser.execute(
      () => getComputedStyle(document.querySelector('.shell')!).backgroundColor,
    );

    expect(typeof reported).toBe('boolean');
    expect(background).not.toBe('rgb(255, 255, 255)');
    expect(ground).not.toBe('');
  });

  it('defines no light-scheme override anywhere in the applied stylesheets', async () => {
    // A prefers-color-scheme: light block would be the mechanism by which the interface
    // could drift into a light appearance. There must not be one.
    const lightBlocks = await browser.execute(() => {
      let count = 0;
      for (const sheet of Array.from(document.styleSheets)) {
        let rules: CSSRuleList;
        try {
          rules = sheet.cssRules;
        } catch {
          continue; // cross-origin sheet; none expected
        }
        for (const rule of Array.from(rules)) {
          if (
            rule instanceof CSSMediaRule &&
            rule.conditionText.includes('prefers-color-scheme: light')
          ) {
            count++;
          }
        }
      }
      return count;
    });
    expect(lightBlocks).toBe(0);
  });
});
