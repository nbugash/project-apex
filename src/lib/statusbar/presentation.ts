import type { ConnectionState } from '../ipc';

/// How each connection state appears.
///
/// In its own module so the component and its tests share one definition. The test used to
/// keep a copy "so it fails when the component changes" — which is backwards: a copy cannot
/// notice a change in the original, and that is why `Retrying` reached the status bar
/// untested and would have thrown.
///
/// FR-012 / SC-007: every state carries an icon **and** a label, so it is never encoded by
/// colour alone and survives a greyscale display.
export const PRESENTATION = {
  unknown: { icon: 'ph-question', label: 'Unknown' },
  connecting: { icon: 'ph-circle-dashed', label: 'Connecting' },
  connected: { icon: 'ph-plugs-connected', label: 'Connected' },
  disconnected: { icon: 'ph-plugs', label: 'Offline' },
} as const;

export interface Presented {
  icon: string;
  label: string;
}

/**
 * Present a connection state.
 *
 * `Retrying` carries data, so it arrives as an object rather than a string; indexing the
 * record with it yields undefined, and the status bar would throw on `.icon` — the state
 * that reports a problem would itself be the one that breaks.
 *
 * The countdown is shown because "Reconnecting" with no sense of progress is
 * indistinguishable from a hang, which is the complaint F000's hidden window taught this
 * project to take seriously.
 */
/** Bytes as a short human string. Whole units: a progress figure that jitters through decimal
 *  places draws the eye to the noise rather than the movement. */
function size(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} kB`;
  return `${bytes} B`;
}

export function present(state: ConnectionState): Presented {
  if (typeof state === 'object' && state !== null && 'deploying' in state) {
    const { sent, total } = state.deploying;
    // The count is the point. "Installing" on its own is indistinguishable from a stall, which
    // is the same reason the transferring state carries bytes at all.
    const progress = total > 0 ? ` ${size(sent)} of ${size(total)}` : '';
    return { icon: 'ph-download-simple', label: `Installing engine${progress}` };
  }
  if (typeof state === 'object' && state !== null && 'retrying' in state) {
    const { attempt, next_in_secs } = state.retrying;
    const when = next_in_secs <= 1 ? 'now' : `in ${next_in_secs}s`;
    return { icon: 'ph-arrows-clockwise', label: `Reconnecting ${when} (attempt ${attempt})` };
  }
  // A state this build does not recognise renders as Unknown rather than blanking the bar.
  // A newer core reporting a state an older interface has not heard of must degrade, not
  // break.
  return PRESENTATION[state] ?? PRESENTATION.unknown;
}
