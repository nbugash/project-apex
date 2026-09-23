import './app.css';
import { mount } from 'svelte';
import Window from './lib/shell/Window.svelte';
import { shellState } from './lib/state.svelte';
import { onConnectionChanged, onWorkspaceChanged, sessionGet, shellReady } from './lib/ipc';
import { CONTENT_PRESENTATION, MAINTENANCE_PRESENTATION } from './lib/statusbar/presentation';

/**
 * Startup order is load-bearing.
 *
 * The window is created hidden and shown only by the readiness signal, which is what makes
 * "no unstyled frame" (FR-020) a structural property. The corollary is unforgiving: anything
 * that can block before the signal can leave the application invisible forever, with nothing
 * reported anywhere a user can see.
 *
 * So the signal is sent as soon as the styled DOM has painted, and BEFORE any optional
 * wiring. Event subscriptions are not a precondition for showing a window; a shell with a
 * stale status bar is a defect, a shell nobody can see is unusable. An awaited subscription
 * that hangs rather than rejects would otherwise never reach a `finally`.
 */
async function start(): Promise<void> {
  try {
    const session = await sessionGet();
    shellState.session = session;
    shellState.workspace = session.workspace;
    mount(Window, { target: document.getElementById('app')!, props: { session } });
  } catch (error) {
    console.error('shell startup failed before render', error);
  }

  // Two frames: the first commits the styled DOM, the second guarantees it has painted.
  //
  // Raced against a deadline, because a HIDDEN window produces no animation frames. Waiting
  // on rAF alone deadlocks: no frames until the window is shown, and the window is not shown
  // until this resolves. The frames are the better signal when they arrive; the deadline is
  // what stops their absence being fatal.
  const painted = new Promise<void>((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
  );
  const deadline = new Promise<void>((resolve) => setTimeout(resolve, 150));
  await Promise.race([painted, deadline]);

  await shellReady().catch((e) => console.error('could not show the window', e));

  // Optional wiring, after the window is visible. Both events fire once with their current
  // value, so nothing here polls.
  onConnectionChanged((state) => {
    shellState.connection = state;
  }).catch((e) => console.error('connection events unavailable', e));

  onWorkspaceChanged((workspace) => {
    shellState.workspace = workspace;
  }).catch((e) => console.error('workspace events unavailable', e));
}

// A test seam, development builds only.
//
// The end-to-end greyscale gate (SC-016) has to check **every** published state, and the
// application never renders all six at once. Exposing the shipped maps lets the gate read the
// real definitions rather than a fixture copy — which is the mistake F001 recorded, where a test
// kept its own copy of a state map and passed while the component crashed.
//
// Guarded by `import.meta.env.DEV`, so it is absent from a release bundle and cannot become a
// production surface by accident, exactly as `stub_set_connection` is `#[cfg(debug_assertions)]`.
if (import.meta.env.DEV) {
  const w = window as unknown as Record<string, unknown>;
  w.__APEX_CONTENT_PRESENTATION__ = CONTENT_PRESENTATION;
  w.__APEX_MAINTENANCE_PRESENTATION__ = MAINTENANCE_PRESENTATION;
}

void start();
