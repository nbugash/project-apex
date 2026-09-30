/// What the interface reads to know it is offline, and what is held locally.
///
/// One store for the window, for the reason the git store is one: a per-component subscription
/// means a background tab either misses updates or pays for its own, and the first of those is a
/// status bar that quietly stops reporting.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { shellState } from '../state.svelte';

const ENGINE_EVENT = 'apex:notification';
/// Sent by the core after a held save and after a reconciliation: the two moments the pending set
/// changes while the connection state does not.
export const PENDING_CHANGED = 'offline/onPendingChanged';

export interface PendingFile {
  relativePath: string;
  /// Whether the client can merge this file as text. `false` means it will prompt on reconnection
  /// whatever the host did (FR-025a), which is worth saying before the reconnection rather than
  /// during it.
  mergeable: boolean;
}

export interface OfflineStatus {
  connected: boolean;
  pending: PendingFile[];
}

export function emptyStatus(): OfflineStatus {
  return { connected: false, pending: [] };
}

/// The window's offline state.
///
/// **Never re-detects whether the client is connected** (FR-001, A-RECONNECT). The value comes
/// from `offline_status`, which the core derives from the connection state F001 publishes; the
/// *trigger* for reading it again is a change in that same published state. So there is one
/// authority on the fact and one authority on when it changed, which is what stops an offline
/// indicator from disagreeing with the status bar beside it.
///
/// A failed request is not evidence of anything here. `refresh` cannot fail for connection
/// reasons -- `offline_status` reads the projection and contacts nothing -- and if it fails for
/// any other reason the previous state stays, because reporting "offline" on a failed read would
/// be exactly the inference this store exists to avoid.
export class OfflineStore {
  #status = $state<OfflineStatus>(emptyStatus());
  #started = false;

  get connected(): boolean {
    return this.#status.connected;
  }

  get pending(): readonly PendingFile[] {
    return this.#status.pending;
  }

  /** Whether this path has work the host has not seen, for the editor's held-locally mark. */
  isHeldLocally(path: string): boolean {
    return this.#status.pending.some((p) => p.relativePath === path);
  }

  /** Read what the core already holds. Cheap, and correct with no connection. */
  async refresh(): Promise<void> {
    try {
      this.#status = await invoke<OfflineStatus>('offline_status');
    } catch {
      // The previous state stands. See the class note: a failed read is not evidence of an
      // outage, and treating it as one is how this indicator would start lying.
    }
  }

  /// Subscribe once for the window.
  ///
  /// Two triggers, both published by the core, neither a probe. `shellState.connection`, which
  /// `main.ts` sets from the core's connection events, covers `connected`; `offline/onPendingChanged`
  /// covers `pending`, which a held save or a reconciliation changes without any connection
  /// change. No timer: A-RECONNECT records that a second detector of a published state is the
  /// defect, and F011 paid for it when the git watch and the workspace watch answered different
  /// questions about one repository. The frame is ignored beyond its method, as the git store
  /// ignores `git/onStatusUpdate`'s: the state is read back through `offline_status`.
  start(): void {
    if (this.#started) return;
    this.#started = true;
    $effect(() => {
      // Read so the effect depends on it; the value itself comes from the core.
      void shellState.connection;
      void this.refresh();
    });
    $effect(() => {
      // Awaited before unlistening, so a teardown that races registration cannot leak a listener.
      const pending = listen<{ method: string }>(ENGINE_EVENT, (event) => {
        if (event.payload.method === PENDING_CHANGED) void this.refresh();
      });
      return () => {
        void pending.then((unlisten) => unlisten()).catch(() => {});
      };
    });
  }
}
