/**
 * Where the panel sends what a person does to a task.
 *
 * A seam rather than a direct `invoke`, for two reasons. The panel's encoding is testable without
 * a Tauri host -- which is what SC-007's client half needs, since a keystroke carrying a control
 * byte has to leave here byte for byte. And the branch T078 rests on, between interrupting a task
 * with a byte and interrupting it with a signal, is a decision this application makes; it belongs
 * somewhere a test can watch it being made.
 */

import { invoke } from '@tauri-apps/api/core';
import { encodeBase64 } from './wire';

/// The three signals §4.8 admits. Nothing else reaches a syscall (see the engine's dispatch).
export type TerminateSignal = 'SIGINT' | 'SIGTERM' | 'SIGKILL';

export interface TaskSink {
  /// Start a task, and answer whether it started.
  ///
  /// The only one of these that awaits, because `execution/runTask` is a request and its
  /// refusals matter: a command that does not exist, an identity already running, a workspace
  /// whose root has vanished. Reporting "started" for any of those would leave a terminal
  /// waiting for output that is never coming.
  ///
  /// No workspace argument: the core knows which workspace it registered, and an interface
  /// that named one would be choosing where a command runs (Principle VI).
  run(taskId: string, command: string[], cols: number, rows: number): Promise<boolean>;
  /// Bytes to the task's input, base64 as the wire carries them.
  writeStdin(taskId: string, data: Uint8Array): void;
  resize(taskId: string, cols: number, rows: number): void;
  terminate(taskId: string, signal: TerminateSignal): void;
}

/// Reported once, not once per keystroke.
///
/// `writeStdin` and `resize` are notifications: §4.2 gives them no response, so a failure here is
/// a transport fault rather than a refusal by the engine, and there is nothing a person could do
/// about it. A dialog over a dropped keystroke helps nobody, and one per keystroke is worse.
///
/// **What it currently reports is that the command does not exist.** The client's remote path is
/// not composed end to end for any feature: `RemoteWorkspaceProvider` is built, tested and never
/// constructed either, and no concrete `RequestSender` exists in `client/core`. So this says so
/// out loud rather than swallowing the rejection, because a silently discarded keystroke and a
/// working one look identical from here.
let reported = false;
function reportOnce(method: string, reason: unknown): void {
  if (reported) return;
  reported = true;
  console.warn(`[apex] ${method} did not reach the engine: ${String(reason)}`);
}

const overIpc: TaskSink = {
  async run(taskId, command, cols, rows) {
    recordForAutomation('run', taskId, command.join(' '));
    try {
      await invoke('task_run', { taskId, command, cols, rows });
      return true;
    } catch (e) {
      // Surfaced every time rather than once: a start that failed is a single event with a
      // cause the developer can act on, which is the opposite of a dropped keystroke.
      console.warn(`[apex] execution/runTask refused: ${String(e)}`);
      return false;
    }
  },
  writeStdin(taskId, data) {
    const encoded = encodeBase64(data);
    recordForAutomation('writeStdin', taskId, encoded);
    void invoke('task_write_stdin', { taskId, data: encoded }).catch((e) =>
      reportOnce('execution/writeStdin', e),
    );
  },
  resize(taskId, cols, rows) {
    recordForAutomation('resize', taskId, `${cols}x${rows}`);
    void invoke('task_resize', { taskId, cols, rows }).catch((e) =>
      reportOnce('execution/resizePty', e),
    );
  },
  terminate(taskId, signal) {
    recordForAutomation('terminate', taskId, signal);
    void invoke('task_terminate', { taskId, signal }).catch((e) =>
      reportOnce('execution/terminate', e),
    );
  },
};

/// What the panel sent, for the end-to-end suite to read back.
///
/// Recorded only under automation, on the same terms as the mounted terminal: a page that logged
/// every keystroke somewhere readable would be doing what this product exists to avoid. What a
/// developer types into a terminal is frequently a credential.
function recordForAutomation(method: string, taskId: string, detail: string): void {
  if (!(import.meta.env.DEV || navigator.webdriver === true)) return;
  const w = window as unknown as { __apexSent?: Array<Record<string, string>> };
  w.__apexSent = w.__apexSent ?? [];
  w.__apexSent.push({ method, taskId, detail });
}

let active: TaskSink = overIpc;

export function taskSink(): TaskSink {
  return active;
}

/// Swap the sink. For tests, which have no Tauri host; returns the previous one so a test can
/// put it back rather than leaving the next one talking to a recorder.
export function setTaskSink(sink: TaskSink): TaskSink {
  const previous = active;
  active = sink;
  return previous;
}
