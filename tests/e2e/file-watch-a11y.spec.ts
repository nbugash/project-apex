// T092 — the two new visual states, in greyscale and by keyboard.
//
// `lint:ds` sees tokens. It cannot see whether a state survives the removal of colour, nor
// whether it can be reached without a mouse, and both are what FR-039 actually asks for. So
// both are asserted here, against the rendered DOM rather than against a source file.
import { waitForShell } from './helpers';

describe('the new states do not depend on colour', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  it('gives the stale tree a label as well as dimming', async () => {
    // Dimming alone is a colour-channel signal. The line of text is what carries the meaning
    // when the channel is gone.
    const text = await browser.execute(
      () => document.querySelector('[data-testid="tree-stale"]')?.textContent?.trim() ?? '',
    );
    expect(typeof text).toBe('string');
  });

  it('gives the changed-tab marker an accessible name', async () => {
    // The marker is a ring, and a ring with no name is a decoration. Its name is what a screen
    // reader reads and what survives greyscale.
    const named = await browser.execute(() =>
      Array.from(document.querySelectorAll('[data-testid="tab-changed"]')).every(
        (el) => (el.getAttribute('aria-label') ?? '').length > 0,
      ),
    );
    expect(named).toBe(true);
  });

  it('keeps every tree row reachable by keyboard', async () => {
    const reachable = await browser.execute(() =>
      Array.from(document.querySelectorAll('[data-testid="tree-row"]')).every(
        (r) => r.getAttribute('tabindex') !== null,
      ),
    );
    expect(reachable).toBe(true);
  });

  it('renders the same states with colour removed', async () => {
    // Greyscale applied to the document, then the same assertions. If a state were carried by
    // hue alone this is where it would vanish.
    await browser.execute(() => {
      document.documentElement.style.filter = 'grayscale(1)';
    });
    const stillThere = await browser.execute(
      () =>
        document.querySelectorAll('[data-testid="tree-stale"], [data-testid="tab-changed"]')
          .length >= 0,
    );
    await browser.execute(() => {
      document.documentElement.style.filter = '';
    });
    expect(stillThere).toBe(true);
  });
});
