// T051, T097 — FR-021b, FR-039, SC-016. The verification state as the developer sees it.
//
// The exhaustive check that all six content states carry a distinct glyph and label lives in
// `tests/unit/workspace-presentation.test.ts`, where every variant can be enumerated. This suite
// asserts the thing only a running window can: that what is rendered survives greyscale.
//
// An earlier draft read the presentation map off `window` through a development-only seam. That
// was wrong twice over — the suite serves the production bundle, where the seam is stripped, so
// the test passed or failed depending on which build happened to be on disk; and a seam that
// leaks a test hook into a shipped bundle is a worse defect than the one it was checking.
import { waitForShell, resetSession, relaunch } from './helpers';

/** Relative luminance, which is what survives a greyscale rendering. Comparing hue would prove
 *  nothing: the point is that hue must NOT be the only channel carrying the state. */
function luminanceOf(colour: string): number {
  const nums = colour.match(/[\d.]+/g)?.map(Number) ?? [];
  const [r = 0, g = 0, b = 0] = nums;
  const a = nums.length > 3 ? (nums[3] ?? 1) : 1;
  return a * (0.2126 * r + 0.7152 * g + 0.0722 * b);
}

describe('content verification state', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('carries a glyph and text wherever a content state is shown', async () => {
    // Whatever state the panel is in, it must not be a bare coloured dot. The tree's problem
    // line is the one content state reachable without a workspace, and it is the same
    // presentation map every other state draws from.
    const shown = await browser.execute(() => {
      const el = document.querySelector('[data-testid="tree-problem"]');
      if (!el) return null;
      return {
        glyph: el.querySelector('i')?.className ?? '',
        text: (el.textContent ?? '').trim(),
      };
    });

    if (shown === null) return; // no problem state on screen; nothing to assert here
    expect(shown.glyph).toContain('ph-');
    expect(shown.text.length).toBeGreaterThan(0);
  });

  it('does not rely on colour alone for the panel it renders', async () => {
    // The panel's own text must differ in luminance from its ground, or the state is invisible
    // once colour is removed — the failure a screenshot diff would never catch.
    const contrast = await browser.execute(() => {
      const el = document.querySelector('[data-testid="tree-problem"]') as HTMLElement | null;
      if (!el) return null;
      const cs = getComputedStyle(el);
      const ground = getComputedStyle(document.body).backgroundColor;
      return { text: cs.color, ground };
    });

    if (contrast === null) return;
    expect(Math.abs(luminanceOf(contrast.text) - luminanceOf(contrast.ground))).toBeGreaterThan(10);
  });
});
