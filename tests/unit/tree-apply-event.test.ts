import { describe, expect, it } from 'vitest';
import {
  WorkspaceTree,
  type FileChange,
  type Node,
} from '../../client/ui/lib/workspace/tree.svelte';

/// Applying an event must correct the tree, not rebuild it.
///
/// Re-fetching the folder would collapse and reconstruct it, losing the developer's expansion
/// state for a change to one file. That is the requirement (FR-016, US1), not an optimisation.

function tree(nodes: Partial<Node>[]): WorkspaceTree {
  const t = new WorkspaceTree('ws1');
  t.nodes = nodes.map((n) => ({
    name: n.name ?? 'x',
    kind: n.kind ?? 'file',
    size: n.size ?? 0,
    modified: n.modified ?? 0,
    path: n.path ?? '/x',
    depth: n.depth ?? 0,
    expanded: n.expanded ?? false,
    loaded: n.loaded ?? false,
  }));
  return t;
}

const created = (path: string): FileChange => ({
  kind: 'created',
  path,
  isDirectory: false,
  size: 10,
  modified: 5,
});

describe('applying an event', () => {
  it('does not reset expansion state', () => {
    const t = tree([
      { path: '/src', name: 'src', kind: 'directory', expanded: true, loaded: true },
      { path: '/src/a.rs', name: 'a.rs', depth: 1 },
    ]);
    t.applyEvent({ kind: 'modified', path: '/src/a.rs', size: 99, modified: 7 });
    expect(t.nodes.find((n) => n.path === '/src')?.expanded).toBe(true);
  });

  it('places a created file into a folder that has been fetched', () => {
    const t = tree([
      { path: '/src', name: 'src', kind: 'directory', expanded: true, loaded: true },
      { path: '/src/a.rs', name: 'a.rs', depth: 1 },
    ]);
    t.applyEvent(created('/src/b.rs'));
    expect(t.nodes.map((n) => n.path)).toContain('/src/b.rs');
  });

  it('ignores a creation inside a folder nobody has opened', () => {
    // Inventing the folder to put it in would show a partial listing as though complete.
    const t = tree([{ path: '/src', name: 'src', kind: 'directory', loaded: false }]);
    t.applyEvent(created('/src/b.rs'));
    expect(t.nodes).toHaveLength(1);
  });

  it('does not add the same path twice', () => {
    const t = tree([
      { path: '/src', name: 'src', kind: 'directory', expanded: true, loaded: true },
      { path: '/src/a.rs', name: 'a.rs', depth: 1 },
    ]);
    t.applyEvent(created('/src/a.rs'));
    expect(t.nodes.filter((n) => n.path === '/src/a.rs')).toHaveLength(1);
  });

  it('removes a deleted entry and everything under it', () => {
    const t = tree([
      { path: '/src', name: 'src', kind: 'directory', expanded: true, loaded: true },
      { path: '/src/deep', name: 'deep', kind: 'directory', depth: 1 },
      { path: '/src/deep/a.rs', name: 'a.rs', depth: 2 },
    ]);
    t.applyEvent({ kind: 'deleted', path: '/src/deep' });
    expect(t.nodes.map((n) => n.path)).toEqual(['/src']);
  });

  it('moves a renamed entry rather than removing and re-adding it', () => {
    // Cached content survives a known move; it does not survive a delete and a create.
    const t = tree([{ path: '/src/expr.rs', name: 'expr.rs' }]);
    t.applyEvent({ kind: 'renamed', path: '/src/expr.rs', toPath: '/src/expression.rs' });
    expect(t.nodes[0]?.path).toBe('/src/expression.rs');
    expect(t.nodes[0]?.name).toBe('expression.rs');
  });

  it('rewrites every descendant of a renamed directory, bounded to the separator', () => {
    // The whole hazard: renaming `src` must not touch `src-generated`.
    const t = tree([
      { path: '/src', name: 'src', kind: 'directory' },
      { path: '/src/a.rs', name: 'a.rs' },
      { path: '/src/deep/b.rs', name: 'b.rs' },
      { path: '/src-generated/c.rs', name: 'c.rs' },
    ]);
    t.applyEvent({ kind: 'renamed', path: '/src', toPath: '/syntax' });
    const paths = t.nodes.map((n) => n.path);
    expect(paths).toContain('/syntax');
    expect(paths).toContain('/syntax/a.rs');
    expect(paths).toContain('/syntax/deep/b.rs');
    expect(paths).toContain('/src-generated/c.rs');
  });

  it('marks stale on a wholesale invalidation without discarding the tree', () => {
    const t = tree([{ path: '/src', name: 'src', kind: 'directory', loaded: true }]);
    t.invalidateAll();
    expect(t.stale).toBe(true);
    expect(t.nodes).toHaveLength(1);
    expect(t.nodes[0]?.loaded).toBe(false);
  });
});
