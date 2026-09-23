import type { OpenDocumentReference } from '../ipc';

/** Tabs render in declared order; the core guarantees contiguity, and sorting here keeps
 *  rendering correct even if an out-of-order payload ever arrives. */
export function inOrder(documents: OpenDocumentReference[]): OpenDocumentReference[] {
  return [...documents].sort((a, b) => a.order - b.order);
}

/** Keyboard navigation target, or null at either end (FR-018). */
export function neighbour(
  documents: OpenDocumentReference[],
  fromId: string,
  step: 1 | -1,
): OpenDocumentReference | null {
  const ordered = inOrder(documents);
  const i = ordered.findIndex((d) => d.id === fromId);
  if (i < 0) return null;
  return ordered[i + step] ?? null;
}
