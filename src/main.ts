import './app.css';
import { mount } from 'svelte';
import Window from './lib/shell/Window.svelte';
import { onConnectionChanged, sessionGet, shellReady, type ConnectionState } from './lib/ipc';

// The window is created hidden. It is shown only after styles have applied and the first
// render has committed, which is what makes FR-020 a structural guarantee rather than a
// race that usually resolves in our favour (research.md, "Preventing a light or unstyled
// first frame").
async function start() {
  const session = await sessionGet();
  let connection: ConnectionState = 'unknown';

  const app = mount(Window, {
    target: document.getElementById('app')!,
    props: { session, connection },
  });

  await onConnectionChanged((state) => {
    connection = state;
    app.$set?.({ connection: state });
  });

  // Two frames: the first commits the styled DOM, the second guarantees it has painted.
  await new Promise<void>((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
  );
  await shellReady();
}

void start();
