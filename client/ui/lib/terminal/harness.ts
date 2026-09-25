/**
 * The seam the end-to-end suite drives the terminal through.
 *
 * SC-002 is a claim about what the panel **renders**, which can only be checked against a real
 * terminal in a real window. Until a task can be started from the interface there is no way to put
 * bytes in front of that renderer, so the suite writes them in directly. What it reaches is the
 * ordinary API -- `show` and `applyChunk` -- and not a parallel rendering path, which is the
 * difference between a seam and a second implementation that proves nothing about the first.
 *
 * **Registered only under automation.** `navigator.webdriver` is set by WebDriver and by nothing
 * else, so this listener does not exist for a person running the application. That matters: a
 * `window` event that injects content into a terminal would otherwise let any script in the
 * webview forge a build's output, and forged output in a terminal is how somebody is persuaded a
 * command succeeded when it did not.
 */

import { applyChunk, terminals, type ReconnectionSummary } from './terminals.svelte';

/** What the suite sends: a task to show, and base64 bytes to write into it. */
export interface HarnessChunk {
  taskId: string;
  /** Base64, exactly as `execution/onStdout` carries it. */
  data: string;
}

export const OUTPUT_EVENT = 'apex:test:task-output';
export const RECONNECT_EVENT = 'apex:test:reconnected';

function underAutomation(): boolean {
  // `import.meta.env.DEV` covers `npm run dev`; `navigator.webdriver` covers the built app under
  // tauri-driver, which is what the suite actually runs against.
  return import.meta.env.DEV || navigator.webdriver === true;
}

/** Install the seam. Returns the uninstall function, and installs nothing outside automation. */
export function installTerminalHarness(): () => void {
  if (!underAutomation()) return () => {};
  const onOutput = (event: Event) => {
    const detail = (event as CustomEvent<HarnessChunk>).detail;
    if (!detail?.taskId || typeof detail.data !== 'string') return;
    applyChunk(terminals.show(detail.taskId), detail.data);
  };
  // The reconnection summary, which has no other way in until the transport exists: the client
  // never actually loses a connection it never had.
  const onReconnect = (event: Event) => {
    const detail = (event as CustomEvent<ReconnectionSummary>).detail;
    if (!detail?.outcomes) return;
    terminals.reconnected(detail);
  };
  window.addEventListener(OUTPUT_EVENT, onOutput);
  window.addEventListener(RECONNECT_EVENT, onReconnect);
  return () => {
    window.removeEventListener(OUTPUT_EVENT, onOutput);
    window.removeEventListener(RECONNECT_EVENT, onReconnect);
  };
}
