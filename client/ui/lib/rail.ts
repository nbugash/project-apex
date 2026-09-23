// Rail ordering and keyboard navigation. Extracted so the logic is testable rather than
// trapped inside a component.
import type { RailDestination } from './ipc';

/** Mirrors the core's MIN_TOOL_WINDOW_WIDTH. The core rejects anything below it, so
 *  clamping the drag here keeps a resize from generating rejected round trips. */
export const MIN_TOOL_WINDOW_WIDTH = 180;

/** Destinations render in declared order. The core guarantees contiguity; sorting here
 *  keeps rendering correct even if an out-of-order payload ever arrives. */
export function inRailOrder(destinations: RailDestination[]): RailDestination[] {
  return [...destinations].sort((a, b) => a.order - b.order);
}

/** Keyboard navigation target, or null at either end.
 *
 *  Unavailable destinations are skipped rather than focused: they are present so the rail's
 *  proportions match the prototype, but stopping on one would make keyboard navigation feel
 *  broken for no benefit. */
export function nextSelectable(
  destinations: RailDestination[],
  fromId: string | null,
  step: 1 | -1,
): RailDestination | null {
  const ordered = inRailOrder(destinations);
  const start =
    fromId === null
      ? step === 1
        ? -1
        : ordered.length
      : ordered.findIndex((d) => d.id === fromId);
  if (fromId !== null && start < 0) return null;
  for (let i = start + step; i >= 0 && i < ordered.length; i += step) {
    const candidate = ordered[i];
    if (candidate?.available) return candidate;
  }
  return null;
}

/** True when the rail has nothing the user can actually open. */
export function hasNoSelectableDestination(destinations: RailDestination[]): boolean {
  return !destinations.some((d) => d.available);
}
