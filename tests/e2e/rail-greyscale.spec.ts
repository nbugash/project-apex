// T032 — US2. Active and unavailable states stay distinguishable in greyscale (FR-005, SC-004).
import { waitForShell, resetSession, relaunch } from './helpers';

/** Relative luminance of a CSS colour, which is what survives a greyscale rendering.
 *  Comparing hue would prove nothing: the point of this test is that hue is NOT the only
 *  channel carrying the state. */
function luminanceOf(colour: string): number {
  const nums = colour.match(/[\d.]+/g)?.map(Number) ?? [];
  const [r = 0, g = 0, b = 0] = nums;
  const a = nums.length > 3 ? (nums[3] ?? 1) : 1;
  // Composite over the rail's dark ground, so a transparent background reads as the
  // ground rather than as black.
  return a * (0.2126 * r + 0.7152 * g + 0.0722 * b);
}

describe('rail states in greyscale', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  it('distinguishes the active destination by more than colour', async () => {
    const marks = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll('nav.rail [role="tab"]'));
      return buttons.map((b) => {
        const mark = b.querySelector('.mark');
        const cs = mark ? getComputedStyle(mark) : null;
        return {
          active: b.getAttribute('aria-selected') === 'true',
          markBackground: cs?.backgroundColor ?? '',
        };
      });
    });

    const active = marks.filter((m) => m.active);
    const inactive = marks.filter((m) => !m.active);
    expect(active.length).toBe(1);

    // The active mark is a painted shape; the inactive ones are transparent. That is a
    // difference in presence, not in hue, so it survives greyscale and colour blindness
    // alike.
    expect(luminanceOf(active[0]!.markBackground)).toBeGreaterThan(0);
    for (const m of inactive) {
      expect(luminanceOf(m.markBackground)).toBe(0);
    }
  });

  it('separates available from unavailable destinations by luminance', async () => {
    const readings = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll('nav.rail [role="tab"]'));
      return buttons.map((b) => ({
        unavailable: b.getAttribute('aria-disabled') === 'true',
        colour: getComputedStyle(b).color,
        opacity: Number(getComputedStyle(b).opacity),
      }));
    });

    const available = readings.filter((r) => !r.unavailable);
    const unavailable = readings.filter((r) => r.unavailable);
    expect(available.length).toBeGreaterThan(0);
    expect(unavailable.length).toBeGreaterThan(0);

    const effective = (r: (typeof readings)[number]) => luminanceOf(r.colour) * r.opacity;
    const dimmest = Math.min(...available.map(effective));
    const brightest = Math.max(...unavailable.map(effective));

    // A margin, not merely "different": two states a viewer cannot tell apart are not
    // distinguishable, however different the computed numbers are.
    expect(dimmest).toBeGreaterThan(brightest * 1.2);
  });
});
