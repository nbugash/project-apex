// Reactive shell state. Svelte 5 has no `$set` on a mounted component, so live values live
// in a rune-backed module the components read directly.
import type { ConnectionState, SessionSnapshot, WorkspaceReference } from './ipc';

export const shellState = $state({
  connection: 'unknown' as ConnectionState,
  workspace: null as WorkspaceReference | null,
  session: null as SessionSnapshot | null,
});
