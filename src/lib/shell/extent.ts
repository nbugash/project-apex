/** Region sizing rules shared by the splitter and its tests.
 *  Mirrors MIN_REGION_EXTENT in the core; the core rejects anything below it on a live
 *  command, so clamping here keeps a drag from generating rejected round trips. */
export const MIN_REGION_EXTENT = 120;

export function clampExtent(
  value: number,
  min: number = MIN_REGION_EXTENT,
  max: number = Number.POSITIVE_INFINITY,
): number {
  return Math.min(Math.max(value, min), max);
}

/** Pointer delta applies along the region's variable axis; the output region grows upward,
 *  so its vertical delta is inverted. */
export function deltaFor(
  orientation: 'vertical' | 'horizontal',
  movementX: number,
  movementY: number,
): number {
  return orientation === 'vertical' ? movementX : -movementY;
}
