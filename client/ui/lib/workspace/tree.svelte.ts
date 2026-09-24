/// The file tree's state.
///
/// Lazy by construction: a folder's children are requested the first time it is expanded and
/// never again while they remain valid (§10.1, FR-015, FR-016). The store holds what has been
/// fetched; it does not model the repository.

import { invoke } from '@tauri-apps/api/core';

export type EntryKind = 'file' | 'directory';

export interface Entry {
  name: string;
  kind: EntryKind;
  size: number;
  modified: number;
}

export interface Node extends Entry {
  path: string;
  depth: number;
  expanded: boolean;
  /** Whether this folder's children have ever been fetched. */
  loaded: boolean;
}

/** Why the tree cannot be shown. `gone` is not staleness — the workspace itself is absent. */
export type TreeProblem =
  { kind: 'offline' } | { kind: 'gone' } | { kind: 'error'; message: string };

export class WorkspaceTree {
  workspaceId = $state('');
  nodes = $state<Node[]>([]);
  problem = $state<TreeProblem | null>(null);
  /** Folders whose listing is in flight, so a double-click cannot issue two requests. */
  #pending = new Set<string>();

  constructor(workspaceId: string) {
    this.workspaceId = workspaceId;
  }

  async open(): Promise<void> {
    // Opening a workspace fetches only the root listing (FR-014, SC-001).
    await this.#load('/', 0);
  }

  /** Expand or collapse. Collapsing keeps what was fetched, so re-expanding costs nothing. */
  async toggle(path: string): Promise<void> {
    const node = this.nodes.find((n) => n.path === path);
    if (!node || node.kind !== 'directory') return;

    if (node.expanded) {
      node.expanded = false;
      this.nodes = this.nodes.filter((n) => !isDescendant(n.path, path));
      return;
    }
    node.expanded = true;
    if (node.loaded) {
      // Already fetched once. Re-expanding must issue no request (US1.3, FR-016).
      return;
    }
    await this.#load(path, node.depth + 1);
    node.loaded = true;
  }

  async #load(path: string, depth: number): Promise<void> {
    if (this.#pending.has(path)) return;
    this.#pending.add(path);
    try {
      const items = await invoke<Entry[]>('workspace_read_directory', {
        workspaceId: this.workspaceId,
        relativePath: path,
      });
      const children: Node[] = items.map((e) => ({
        ...e,
        path: path === '/' ? `/${e.name}` : `${path}/${e.name}`,
        depth,
        expanded: false,
        loaded: false,
      }));
      const at = path === '/' ? 0 : this.nodes.findIndex((n) => n.path === path) + 1;
      this.nodes =
        path === '/'
          ? children
          : [...this.nodes.slice(0, at), ...children, ...this.nodes.slice(at)];
      this.problem = null;
    } catch (e) {
      this.problem = classify(e);
    } finally {
      this.#pending.delete(path);
    }
  }
}

function isDescendant(path: string, ancestor: string): boolean {
  return path.startsWith(ancestor === '/' ? '/' : `${ancestor}/`) && path !== ancestor;
}

/// A gone workspace is not an outage.
///
/// Offline means possibly stale and still true; gone means the thing being projected does not
/// exist, and continuing to browse it would be fiction rather than a stale fact. The core sends
/// these as typed variants precisely so this does not have to match on a message — a message is
/// something a refactor changes silently.
export function classify(e: unknown): TreeProblem {
  const kind = (e as { kind?: string } | null)?.kind;
  if (kind === 'gone') return { kind: 'gone' };
  if (kind === 'offline') return { kind: 'offline' };
  return { kind: 'error', message: typeof e === 'string' ? e : JSON.stringify(e) };
}
