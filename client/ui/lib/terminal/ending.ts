/**
 * How a task ended, said in words.
 *
 * Pure, and separate from the component that shows it, because what an ending *means* is a rule
 * and a component is a rendering. The rules here are FR-029's, and each of them is a case that
 * looked fine until it was written down.
 */

import type { Ending } from './terminals.svelte';

/// What the badge carries, and what a screen reader is told.
export interface EndingLabel {
  /// The short form, for the tab's badge. Text, so it survives greyscale.
  badge: string;
  /// The full sentence, for the accessible name. A badge reading `1` is not a sentence.
  spoken: string;
  /// Whether this counts as a clean ending. Drives the hue, which is the **second** channel.
  ok: boolean;
}

/// Signals are reported by name, and a name is what is shown.
///
/// The `SIG` prefix is dropped for the badge only: `TERM` fits a tab and `SIGTERM` does not, and
/// the accessible name keeps the full name so nothing is lost where there is room for it.
function short(signal: string): string {
  return signal.startsWith('SIG') ? signal.slice(3) : signal;
}

/**
 * Describe an ending.
 *
 * **A signal is read first, and `128 + n` is never manufactured.** A shell reports a signalled
 * death as `128 + n` because its only channel is an exit status; this protocol spent a field
 * keeping the two apart, and rebuilding the convention one layer up would throw that away.
 *
 * **Both fields or neither is an error, never `Exited 0`.** The protocol says exactly one is
 * present. A frame carrying both is a contradiction and one carrying neither says nothing, and
 * in both cases the honest report is that the ending is unknown -- reporting a clean exit would
 * be inventing the one answer a developer most wants to be able to trust.
 */
export function describeEnding(ending: Ending): EndingLabel {
  const hasCode = ending.exitCode !== null && ending.exitCode !== undefined;
  const hasSignal = ending.signal !== null && ending.signal !== undefined && ending.signal !== '';

  if (hasCode && hasSignal) {
    return { badge: '?', spoken: 'ended with a contradictory status', ok: false };
  }
  if (hasSignal) {
    const name = ending.signal as string;
    return { badge: short(name), spoken: `killed by ${name}`, ok: false };
  }
  if (!hasCode) {
    return { badge: '?', spoken: 'ended for an unknown reason', ok: false };
  }
  const code = ending.exitCode as number;
  return { badge: String(code), spoken: `exited ${code}`, ok: code === 0 };
}
