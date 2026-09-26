/**
 * The real route: engine frames arriving over the Tauri event bridge.
 *
 * The core forwards every frame the engine sent unasked on one event, uninterpreted. Routing on
 * the method happens here because this is the layer that knows what a terminal is; the core has
 * no decision to make about a stream it does not render.
 *
 * # Why the frame is parsed here and not in the core
 *
 * `applyChunk` already reads `data` and decodes it. A second parser in the core would be a
 * second place that has to agree with the wire, and two parsers that must stay in step is how a
 * client and an engine come to disagree about a field name while each looks correct alone.
 *
 * # This is not the harness
 *
 * `harness.ts` injects bytes from the suite and exists only under automation. This listener is
 * the product path and runs always. They meet at `applyChunk`, which is deliberate: the suite
 * drives the same rendering the engine does, so what it proves about the renderer is true of
 * the real one.
 */

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { applyChunk, terminals } from './terminals.svelte';

/** The single event the core forwards every engine-initiated frame on. */
export const NOTIFICATION_EVENT = 'apex:notification';

interface Notification {
  method: string;
  /** The whole JSON-RPC frame, as it came off the wire. */
  body: string;
}

/** The fields this layer reads. Everything else in the frame is left alone. */
interface OutputParams {
  task_id?: string;
  data?: string;
}

interface ExitParams {
  task_id?: string;
  exit_code?: number | null;
  signal?: string | null;
}

function paramsOf<T>(body: string): T | null {
  try {
    const frame = JSON.parse(body) as { params?: T };
    return frame.params ?? null;
  } catch {
    // A frame that will not parse is one this layer cannot act on. Dropped rather than guessed
    // at: inventing bytes here would put output in a terminal that no process produced, which
    // is the one failure a build log must not have.
    return null;
  }
}

/**
 * Start listening. Returns the unlisten function.
 *
 * Async because the Tauri bridge is, and the caller is an effect that may be torn down before
 * the listener is registered -- so the returned promise has to be awaited before unlistening,
 * or the listener outlives the component that asked for it.
 */
export function routeNotification(method: string, body: string): void {
  switch (method) {
    case 'execution/onStdout':
    case 'execution/onStderr': {
      const params = paramsOf<OutputParams>(body);
      if (!params?.task_id || typeof params.data !== 'string') return;
      // `show` rather than `panel`, so output for a task this client has not displayed yet
      // creates its panel. A reattached task produces bytes before anything on screen has
      // asked for it.
      applyChunk(terminals.show(params.task_id), params.data);
      return;
    }
    case 'execution/onExit': {
      const params = paramsOf<ExitParams>(body);
      if (!params?.task_id) return;
      terminals.ended(params.task_id, {
        exitCode: params.exit_code ?? null,
        signal: params.signal ?? null,
      });
      return;
    }
    default:
      // Every other method belongs to a feature that routes it elsewhere. Ignored rather than
      // warned about: the core forwards everything on one event precisely so a new method needs
      // no change here, and a warning would make that a nuisance instead.
      return;
  }
}

export async function listenToEngine(): Promise<UnlistenFn> {
  return listen<Notification>(NOTIFICATION_EVENT, (event) =>
    routeNotification(event.payload.method, event.payload.body),
  );
}
