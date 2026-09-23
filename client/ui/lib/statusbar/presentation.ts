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

/// How each content presentation appears (F003, FR-039).
///
/// Extended here rather than in a parallel module, for the reason the header above records: a
/// second copy cannot notice a change in the first. Every state carries an icon **and** a label,
/// so none is encoded by colour alone and all survive a greyscale display — which the adherence
/// lint cannot check, because its rules are about raw hex, raw pixels and font families and have
/// no view of whether a state has a second channel.
export const CONTENT_PRESENTATION = {
  verifying: { icon: 'ph-circle-dashed', label: 'Checking for changes' },
  current: { icon: 'ph-check-circle', label: 'Up to date' },
  unverified: { icon: 'ph-warning-circle', label: 'Could not verify' },
  possiblyStale: { icon: 'ph-clock-countdown', label: 'May be out of date' },
  unavailable: { icon: 'ph-cloud-slash', label: 'Not available offline' },
  gone: { icon: 'ph-trash', label: 'Workspace no longer exists' },
} as const;

export type ContentState = keyof typeof CONTENT_PRESENTATION;

/// How each rendered maintenance phase appears (F003, FR-018a).
///
/// Only the three phases the interface shows. `Idle`, `Checking` and `Ready` order the work and
/// are never rendered — and migrating stays distinguishable from evicting, because FR-018a
/// requires a state saying a *migration* is running and SC-013a asserts on migration reports
/// specifically. Collapsing them into one "maintaining" state would make that criterion
/// unmeasurable.
export const MAINTENANCE_PRESENTATION = {
  migrating: { icon: 'ph-arrows-clockwise', label: 'Updating cached files' },
  rebuilding: { icon: 'ph-arrow-counter-clockwise', label: 'Rebuilding cached files' },
  evicting: { icon: 'ph-broom', label: 'Reclaiming disk space' },
} as const;

export type MaintenanceState = keyof typeof MAINTENANCE_PRESENTATION;

/** Present a content state, degrading rather than breaking on one this build does not know. */
export function presentContent(state: string): Presented {
  return (
    CONTENT_PRESENTATION[state as ContentState] ?? {
      icon: 'ph-question',
      label: 'Unknown',
    }
  );
}

/** Present a maintenance phase, or `null` when the phase is not one the interface shows. */
export function presentMaintenance(
  phase: string,
  progress?: { from: number; to: number },
): Presented | null {
  const base = MAINTENANCE_PRESENTATION[phase as MaintenanceState];
  if (!base) return null;
  // The versions are shown for the same reason the deployment progress carries bytes: a label
  // with no sense of movement is indistinguishable from a stall.
  if (phase === 'migrating' && progress) {
    return { icon: base.icon, label: `${base.label} (v${progress.from} to v${progress.to})` };
  }
  return base;
}
