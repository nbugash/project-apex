/**
 * Routing a file event to the buffer it concerns.
 *
 * # What is real here and what is not
 *
 * The client half is real: an event names a path, the buffer for that path decides what it
 * means, and A-WRITEECHO's hash comparison is what keeps the developer's own save from being
 * reported back to them as somebody else's change.
 *
 * The engine half **is** wired, as of F011. It was not before: this module listened for a
 * Tauri event named `workspace:file-event` that nothing anywhere emitted, so no host change
 * ever reached a buffer or the tree. The core forwards every engine-initiated frame on one
 * event, `apex:notification`, routed by method -- so the fix was to listen where the frames
 * actually arrive rather than to emit a second event from the core.
 *
 * F011 depends on this only indirectly: the engine refreshes git status from these same
 * events on its own side (A-GITNUDGE), so marks appear without it. What does not appear
 * without it is the **row** for a file created while the application is running.
 */

import { listen } from '@tauri-apps/api/event';
import { buffers, onFileEvent } from './buffers.svelte';

export interface HostFileEvent {
  /// `/src/main.rs`, as §4.8 spells it.
  relative_path: string;
  /// `created`, `modified`, `deleted`, `renamed`.
  event: string;
  /// Present on created and modified only; a deleted path has nothing to describe.
  size?: number;
  modified?: number;
  kind?: string;
  /// Where a rename went.
  to_path?: string;
}

/** The single event the core forwards every engine-initiated frame on. */
const ENGINE_EVENT = 'apex:notification';

/// Every host file event that reached the webview, for the suite to assert on.
///
/// Recorded for the same reason `tree.svelte.ts` records listings: this suite contains watch
/// assertions of the form `expect(n).toBeGreaterThanOrEqual(0)`, which pass for a client that
/// receives nothing at all -- and did, for two features.
function recordMethod(method: string): void {
  const w = window as unknown as { __apexMethods?: string[] };
  w.__apexMethods = w.__apexMethods ?? [];
  w.__apexMethods.push(method);
}

function recordHostEvents(events: HostFileEvent[]): void {
  const w = window as unknown as { __apexHostEvents?: HostFileEvent[] };
  w.__apexHostEvents = w.__apexHostEvents ?? [];
  w.__apexHostEvents.push(...events);
}

/// Consumers of a wholesale invalidation (§10.4, FR-017).
///
/// It was routed nowhere: the engine emits `workspace/invalidateAll`, no client listener
/// existed, and the suite's synthetic `apex:test:invalidate-all` event had none either -- so
/// `file-watch.spec.ts` dispatched into silence and asserted a tautology about the result.
const onInvalidate: Array<() => void> = [];

export function onWorkspaceInvalidated(fn: () => void): () => void {
  onInvalidate.push(fn);
  return () => {
    const at = onInvalidate.indexOf(fn);
    if (at >= 0) onInvalidate.splice(at, 1);
  };
}

export function invalidateAll(): void {
  for (const fn of onInvalidate) fn();
}

/// Extra consumers of a batch, beyond the open buffers.
///
/// The tree is one: a file created on the host has no row until something inserts it, and the
/// buffers know nothing about rows. A list rather than a second listener so that both see the
/// same batch in the same order.
const alsoDeliver: Array<(events: HostFileEvent[]) => void> = [];

export function onHostFileEvents(fn: (events: HostFileEvent[]) => void): () => void {
  alsoDeliver.push(fn);
  return () => {
    const at = alsoDeliver.indexOf(fn);
    if (at >= 0) alsoDeliver.splice(at, 1);
  };
}

/// Deliver one event to the buffer it names, if that file is open.
///
/// A file nobody has open is not this module's business: the tree handles the projection, and a
/// buffer that does not exist has nothing to be told.
export async function deliver(events: HostFileEvent[]): Promise<void> {
  for (const fn of alsoDeliver) fn(events);
  for (const e of events) {
    const buffer = buffers.get(e.relative_path);
    if (!buffer) continue;
    await onFileEvent(buffer, e.event === 'deleted' ? 'deleted' : 'changed');
  }
}

/// Start listening. Returns a function that stops.
export function startFileEvents(): () => void {
  const stops: Array<() => void> = [];

  void listen<{ method: string; body: string }>(ENGINE_EVENT, (m) => {
    recordMethod(m.payload.method);
    if (m.payload.method === 'workspace/invalidateAll') {
      invalidateAll();
      return;
    }
    if (m.payload.method !== 'workspace/onFileEvent') return;
    let events: HostFileEvent[] = [];
    try {
      const frame = JSON.parse(m.payload.body) as { params?: { events?: HostFileEvent[] } };
      events = frame.params?.events ?? [];
    } catch {
      // A frame that will not parse is one this layer cannot act on. Dropped rather than
      // guessed at: inventing an event here would tell a buffer its file changed when
      // nothing said so.
      return;
    }
    recordHostEvents(events);
    void deliver(events);
  }).then((un) => stops.push(un));

  if (import.meta.env.DEV || navigator.webdriver === true) {
    const handler = (e: Event): void => {
      const detail = (e as CustomEvent<{ events?: HostFileEvent[] }>).detail;
      void deliver(detail?.events ?? []);
    };
    window.addEventListener('apex:test:file-event', handler);
    stops.push(() => window.removeEventListener('apex:test:file-event', handler));
    const invalidated = (): void => invalidateAll();
    window.addEventListener('apex:test:invalidate-all', invalidated);
    stops.push(() => window.removeEventListener('apex:test:invalidate-all', invalidated));
  }

  return () => stops.forEach((s) => s());
}
