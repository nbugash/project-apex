import { describe, expect, it } from 'vitest';
import { classify } from '../../client/ui/lib/workspace/tree.svelte';

describe('tree problem classification (FR-038)', () => {
  it('separates a gone workspace from an outage', () => {
    // These lead to opposite responses. An outage is temporary and the projection is still
    // true; a deleted workspace means the thing being projected does not exist.
    expect(classify({ kind: 'gone' })).toEqual({ kind: 'gone' });
    expect(classify({ kind: 'offline' })).toEqual({ kind: 'offline' });
    expect(classify({ kind: 'gone' })).not.toEqual(classify({ kind: 'offline' }));
  });

  it('reads the typed variant rather than matching on a message', () => {
    // The core sends a tagged variant so this never has to parse prose. A message match would
    // break silently the day someone rewords the error — and the symptom would be a deleted
    // workspace presenting as an outage, which is exactly the confusion -32009 exists to end.
    expect(classify({ kind: 'gone', detail: 'anything at all' }).kind).toBe('gone');
    expect(classify('the workspace root no longer exists').kind).toBe('error');
  });

  it('falls back to an error rather than guessing', () => {
    expect(classify({ kind: 'not_found' }).kind).toBe('error');
    expect(classify(null).kind).toBe('error');
  });
});
