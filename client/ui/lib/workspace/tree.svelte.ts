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
  /// Regions whose contents must be re-read before they are trusted.
  ///
  /// Distinct from content validity, which is a hash comparison. A stale region is still
  /// shown — dimmed, following the prototype's own treatment — because a tree that vanished
  /// on a branch switch would be worse than one that says it may have moved on (FR-017,
  /// FR-026).
  stale = $state(false);
  /** Folders whose listing is in flight, so a double-click cannot issue two requests. */
  #pending = new Set<string>();

  constructor(workspaceId: string) {
    this.workspaceId = workspaceId;
  }

  async open(): Promise<void> {
    // Opening a workspace fetches only the root listing (FR-014, SC-001).
    await this.#load('/', 0);
  }

  /** Every folder currently expanded, which is half of what the engine is asked to watch. */
  expandedFolders(): string[] {
    return this.nodes.filter((n) => n.kind === 'directory' && n.expanded).map((n) => n.path);
  }

  /// Apply one change in place.
  ///
  /// In place, and that is the requirement rather than an optimisation: re-fetching the folder
  /// would collapse and rebuild it, losing the developer's expansion state for a change to one
  /// file (FR-016, US1).
  applyEvent(event: FileChange): void {
    const parent = parentOf(event.path);
    switch (event.kind) {
      case 'created': {
        if (this.nodes.some((n) => n.path === event.path)) return;
        // Only into a folder that has been fetched. A creation inside a folder nobody has
        // opened is not something to show, and inventing the folder to put it in would be.
        const at = this.#insertionPoint(parent);
        if (at === null) return;
        const depth = (this.nodes.find((n) => n.path === parent)?.depth ?? -1) + 1;
        const node: Node = {
          name: nameOf(event.path),
          kind: event.isDirectory ? 'directory' : 'file',
          size: event.size,
          modified: event.modified,
          path: event.path,
          depth,
          expanded: false,
          loaded: false,
        };
        this.nodes = [...this.nodes.slice(0, at), node, ...this.nodes.slice(at)];
        return;
      }
      case 'modified': {
        this.nodes = this.nodes.map((n) =>
          n.path === event.path ? { ...n, size: event.size, modified: event.modified } : n,
        );
        return;
      }
      case 'deleted': {
        this.nodes = this.nodes.filter(
          (n) => n.path !== event.path && !isDescendant(n.path, event.path),
        );
        return;
      }
      case 'renamed': {
        // The entry moves rather than being removed and re-added, so cached content survives
        // (FR-021). The separator bound is what stops renaming `src` touching `src-generated`.
        const to = event.toPath;
        this.nodes = this.nodes.map((n) => {
          if (n.path === event.path) return { ...n, path: to, name: nameOf(to) };
          if (isDescendant(n.path, event.path)) {
            const moved = to + n.path.slice(event.path.length);
            return { ...n, path: moved, name: nameOf(moved) };
          }
          return n;
        });
        return;
      }
    }
  }

  /// A wholesale invalidation (§10.4, FR-017).
  ///
  /// Marks stale and re-reads nothing. A burst of listings at the moment a link has just
  /// proved unreliable is the worst time to issue one (FR-026a).
  invalidateAll(): void {
    this.stale = true;
    this.nodes = this.nodes.map((n) => ({ ...n, loaded: false }));
  }

  #insertionPoint(parent: string): number | null {
    if (parent === '/') return this.nodes.length === 0 ? null : this.nodes.length;
    const at = this.nodes.findIndex((n) => n.path === parent);
    const folder = at < 0 ? undefined : this.nodes[at];
    // Only into a folder whose children have been fetched. Inserting into one nobody has
    // opened would show a partial listing as though it were complete.
    if (!folder?.loaded) return null;
    let i = at + 1;
    while (i < this.nodes.length && isDescendant(this.nodes[i]?.path ?? '', parent)) i += 1;
    return i;
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

/** One change, as the tree sees it. Carries no content — an event never does (FR-013). */
export type FileChange =
  | { kind: 'created'; path: string; isDirectory: boolean; size: number; modified: number }
  | { kind: 'modified'; path: string; size: number; modified: number }
  | { kind: 'deleted'; path: string }
  | { kind: 'renamed'; path: string; toPath: string };

function nameOf(path: string): string {
  const at = path.lastIndexOf('/');
  return at < 0 ? path : path.slice(at + 1);
}

function parentOf(path: string): string {
  const at = path.lastIndexOf('/');
  return at <= 0 ? '/' : path.slice(0, at);
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
