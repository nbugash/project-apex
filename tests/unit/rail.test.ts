import { describe, expect, it } from 'vitest';
import { hasNoSelectableDestination, inRailOrder, nextSelectable } from '../../client/ui/lib/rail';
import type { RailDestination } from '../../client/ui/lib/ipc';

const dest = (id: string, order: number, available = true): RailDestination => ({
  id,
  label: id,
  icon: `ph-${id}`,
  available,
  order,
});

// Mirrors the catalogue's shape: the first is available, several are not.
const rail = [dest('files', 0), dest('vcs', 1, false), dest('search', 2), dest('run', 3, false)];

describe('rail ordering', () => {
  it('renders in declared order regardless of array order', () => {
    expect(inRailOrder([...rail].reverse()).map((d) => d.id)).toEqual([
      'files',
      'vcs',
      'search',
      'run',
    ]);
  });
});

describe('keyboard navigation (FR-007)', () => {
  it('skips unavailable destinations rather than stopping on them', () => {
    // 'vcs' sits between 'files' and 'search' and is unavailable.
    expect(nextSelectable(rail, 'files', 1)?.id).toBe('search');
  });

  it('walks backwards past unavailable destinations too', () => {
    expect(nextSelectable(rail, 'search', -1)?.id).toBe('files');
  });

  it('stops at either end rather than wrapping', () => {
    expect(nextSelectable(rail, 'search', 1)).toBeNull();
    expect(nextSelectable(rail, 'files', -1)).toBeNull();
  });

  it('enters the rail at the first selectable destination', () => {
    expect(nextSelectable(rail, null, 1)?.id).toBe('files');
  });

  it('returns null for an unknown destination', () => {
    expect(nextSelectable(rail, 'ghost', 1)).toBeNull();
  });

  it('reports a rail with nothing selectable', () => {
    expect(hasNoSelectableDestination([dest('a', 0, false)])).toBe(true);
    expect(hasNoSelectableDestination(rail)).toBe(false);
  });
});
