/**
 * Routing a file event to the buffer it concerns.
 *
 * # What is real here and what is not
 *
 * The client half is real: an event names a path, the buffer for that path decides what it
 * means, and A-WRITEECHO's hash comparison is what keeps the developer's own save from being
 * reported back to them as somebody else's change.
 *
 * The **engine half is not wired**. `workspace/onFileEvent` exists in §4.8 and the engine's
 * watcher emits it, but nothing in the client forwards it to the webview: F004 built
 * `file_event_notification.rs` and no caller, so the module is declared and never constructed.
 * That is F004's gap rather than this feature's, and it is recorded rather than patched over —
 * so this listens for the Tauri event that will exist, and, under automation only, for a
 * synthetic one, which is what makes the echo rule testable today.
 */

import { listen } from '@tauri-apps/api/event';
import { buffers, onFileEvent } from './buffers.svelte';

export interface HostFileEvent {
  /// `/src/main.rs`, as §4.8 spells it.
  relative_path: string;
  /// `created`, `modified`, `deleted`, `renamed`.
  event: string;
}

/// Deliver one event to the buffer it names, if that file is open.
///
/// A file nobody has open is not this module's business: the tree handles the projection, and a
/// buffer that does not exist has nothing to be told.
export async function deliver(events: HostFileEvent[]): Promise<void> {
  for (const e of events) {
    const buffer = buffers.get(e.relative_path);
    if (!buffer) continue;
    await onFileEvent(buffer, e.event === 'deleted' ? 'deleted' : 'changed');
  }
}

/// Start listening. Returns a function that stops.
export function startFileEvents(): () => void {
  const stops: Array<() => void> = [];

  void listen<{ events?: HostFileEvent[] }>('workspace:file-event', (m) => {
    void deliver(m.payload?.events ?? []);
  }).then((un) => stops.push(un));

  if (import.meta.env.DEV || navigator.webdriver === true) {
    const handler = (e: Event): void => {
      const detail = (e as CustomEvent<{ events?: HostFileEvent[] }>).detail;
      void deliver(detail?.events ?? []);
    };
    window.addEventListener('apex:test:file-event', handler);
    stops.push(() => window.removeEventListener('apex:test:file-event', handler));
  }

  return () => stops.forEach((s) => s());
}
