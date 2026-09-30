/// The conflicts the last reconciliation left, for the window (US4).
///
/// **Refreshed, never cached.** The core reads each conflict's remote side when the list is built,
/// so a list kept here would be exactly the stale remote the contract exists to prevent. Refreshed
/// on the same two published triggers as `OfflineStore` -- a connection change and
/// `offline/onPendingChanged`, which the core sends after a reconciliation and after a resolution.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { shellState } from '../state.svelte';
import { PENDING_CHANGED } from './state.svelte';
import type { Conflict } from './conflict-presentation';

const ENGINE_EVENT = 'apex:notification';

export type Resolution = { kind: 'text'; text: string } | { kind: 'keepLocal' } | { kind: 'takeRemote' };

export interface ResolveOutcome {
  outcome: 'resolved' | 'conflicted' | 'refused' | 'notAttempted' | 'failed';
  message: string | null;
}

export class ConflictStore {
  #conflicts = $state<Conflict[]>([]);
  #started = false;

  get conflicts(): readonly Conflict[] {
    return this.#conflicts;
  }

  /// A failed listing keeps the previous one. Offline the core refuses to list rather than show a
  /// remote it cannot read, and a panel that vanished on every outage would read as "resolved".
  async refresh(): Promise<void> {
    try {
      this.#conflicts = await invoke<Conflict[]>('conflicts_list');
    } catch {
      // The previous list stands; see above.
    }
  }

  async resolve(c: Conflict, resolution: Resolution): Promise<ResolveOutcome> {
    try {
      const result = await invoke<ResolveOutcome>('conflict_resolve', {
        relativePath: c.relativePath,
        resolution,
        remoteSha256: c.remoteSha256,
      });
      await this.refresh();
      return result;
    } catch (e) {
      return { outcome: 'failed', message: String(e) };
    }
  }

  start(): void {
    if (this.#started) return;
    this.#started = true;
    $effect(() => {
      void shellState.connection;
      void this.refresh();
    });
    $effect(() => {
      const pending = listen<{ method: string }>(ENGINE_EVENT, (event) => {
        if (event.payload.method === PENDING_CHANGED) void this.refresh();
      });
      return () => {
        void pending.then((unlisten) => unlisten()).catch(() => {});
      };
    });
  }
}
