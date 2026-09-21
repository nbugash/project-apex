// T042 — US2. Every tab stays reachable past the strip width (FR-006).
import { openDocuments, waitForShell, relaunch, resetSession } from './helpers';

describe('tab overflow', () => {
  before(async () => {
    // One profile directory is shared across the run, so a spec that assumes defaults
    // says so rather than depending on which spec happened to run before it.
    resetSession();
    await relaunch();
  });

  it('keeps every open document reachable when more are open than fit', async () => {
    await openDocuments(25);
    const tabs = await $$('[role="tab"]');
    expect(tabs.length).toBe(25);

    // The strip scrolls rather than shrinking labels into illegibility.
    const overflowed = await browser.execute(() => {
      const strip = document.querySelector('.strip') as HTMLElement;
      return strip.scrollWidth > strip.clientWidth;
    });
    expect(overflowed).toBe(true);

    // And the overflow list names all of them, so finding one is a single interaction
    // rather than a drag through twenty-four others.
    await $('[aria-label="All open documents"]').click();
    expect(await $$('[role="option"]')).toHaveLength(25);
  });

  it('scrolls a document chosen from the overflow list into view', async () => {
    const options = [...(await $$('[role="option"]'))];
    await options[options.length - 1]!.click();

    const visible = await browser.execute(() => {
      const active = document.querySelector('.tab.active') as HTMLElement;
      const strip = document.querySelector('.strip') as HTMLElement;
      if (!active) return false;
      const a = active.getBoundingClientRect();
      const s = strip.getBoundingClientRect();
      return a.left >= s.left - 1 && a.right <= s.right + 1;
    });
    expect(visible).toBe(true);
  });

  it('does not shrink tab labels to fit', async () => {
    const widths = await browser.execute<number[], []>(() =>
      [...document.querySelectorAll('.tab')].map((t) => (t as HTMLElement).offsetWidth),
    );
    expect(Math.min(...widths)).toBeGreaterThan(40);
  });
});
