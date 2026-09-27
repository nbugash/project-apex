/// The git projection every surface reads, updated rather than polled.
///
/// One store for the window, for the reason the file-event router is one: a per-component
/// subscription means a background tab either misses updates or pays for its own, and the
/// first of those is a tree that quietly stops marking (F004's lesson, A-GITNUDGE's coupling).

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { GitState } from './marker';

/** The single event the core forwards every engine-initiated frame on. */
const ENGINE_EVENT = 'apex:notification';

export interface GitChange {
  path: string;
  status: GitState;
}

/// Three cases, never an optional name.
///
/// git reports a detached head as the literal `(detached)` where a name goes, so an optional
/// string would put that text on the status bar as though it were a branch (FR-019).
export type GitBranch =
  | { kind: 'branch'; name: string }
  | { kind: 'detached'; commit: string }
  | { kind: 'none' };

export interface GitStatus {
  branch: GitBranch;
  changes: GitChange[];
}

export function emptyStatus(): GitStatus {
  return { branch: { kind: 'none' }, changes: [] };
}

/// The window's git state.
///
/// `byPath` is a map rather than a list because the tree asks per row: a list turns rendering
/// a thousand-row tree into a thousand scans of a thousand-entry array, which is the kind of
/// cost that only appears on the repositories where it matters.
export class GitStatusStore {
  #status = $state<GitStatus>(emptyStatus());
  #byPath = $derived(new Map(this.#status.changes.map((c) => [c.path, c.status])));
  #stop: UnlistenFn | null = null;

  get branch(): GitBranch {
    return this.#status.branch;
  }

  get changes(): readonly GitChange[] {
    return this.#status.changes;
  }

  /** The state of one path, or `undefined` for a file with no git state (FR-016). */
  stateOf(path: string): GitState | undefined {
    return this.#byPath.get(path);
  }

  /** Read what the core already holds. Cheap, and correct with no connection (FR-029). */
  async refresh(): Promise<void> {
    try {
      this.#status = await invoke<GitStatus>('git_status');
    } catch {
      // A failed read leaves the previous state exactly as it was. Clearing would say nothing
      // has changed, which is a claim this client cannot make when it could not read.
    }
  }

  /// Subscribe once for the window.
  ///
  /// On the **one** event the core forwards every engine-initiated frame on, filtered by
  /// method. A dedicated Tauri event per method reads better until something adds a method, at
  /// which point the addition is in three places and forgetting the middle one produces silence
  /// rather than an error (`webview_notifications.rs` records this).
  ///
  /// The frame itself is ignored beyond its method: the state is read back through the command,
  /// so there is one path by which the projection is produced rather than two that could
  /// disagree. The core has already committed the replacement by the time this arrives.
  async start(): Promise<void> {
    if (this.#stop) return;
    this.#stop = await listen<{ method: string }>(ENGINE_EVENT, (event) => {
      if (event.payload.method === 'git/onStatusUpdate') void this.refresh();
    });
    await this.refresh();
  }

  stop(): void {
    this.#stop?.();
    this.#stop = null;
  }
}
