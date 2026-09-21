import { describe, expect, it } from 'vitest';
import { inOrder, neighbour } from '../../src/lib/tabs/ordering';
import type { OpenDocumentReference } from '../../src/lib/ipc';

const docs = (n: number): OpenDocumentReference[] =>
  Array.from({ length: n }, (_, i) => ({ id: `d${i}`, display_name: `doc${i}`, order: i }));

describe('tab ordering', () => {
  it('renders in declared order regardless of array order', () => {
    const shuffled = [...docs(4)].reverse();
    expect(inOrder(shuffled).map((d) => d.id)).toEqual(['d0', 'd1', 'd2', 'd3']);
  });

  it('finds the next and previous tab for keyboard navigation (FR-018)', () => {
    const d = docs(3);
    expect(neighbour(d, 'd1', 1)?.id).toBe('d2');
    expect(neighbour(d, 'd1', -1)?.id).toBe('d0');
  });

  it('stops at either end rather than wrapping', () => {
    const d = docs(3);
    expect(neighbour(d, 'd2', 1)).toBeNull();
    expect(neighbour(d, 'd0', -1)).toBeNull();
  });

  it('returns null for an unknown document', () => {
    expect(neighbour(docs(3), 'ghost', 1)).toBeNull();
  });
});
