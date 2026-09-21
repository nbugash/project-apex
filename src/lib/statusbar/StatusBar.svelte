<script lang="ts">
  import type { ConnectionState, WorkspaceReference } from '../ipc';

  interface Props {
    connection: ConnectionState;
    workspace: WorkspaceReference | null;
    persistenceFailed?: boolean;
  }
  let { connection, workspace, persistenceFailed = false }: Props = $props();

  // FR-012 / SC-007: every state carries an icon AND a label, so it is never encoded by
  // colour alone and survives a greyscale display.
  const PRESENTATION: Record<ConnectionState, { icon: string; label: string }> = {
    unknown: { icon: 'ph-question', label: 'Unknown' },
    connecting: { icon: 'ph-circle-dashed', label: 'Connecting' },
    connected: { icon: 'ph-plugs-connected', label: 'Connected' },
    disconnected: { icon: 'ph-plugs', label: 'Offline' },
  };

  let state = $derived(PRESENTATION[connection]);
</script>

<footer class="status" aria-label="Session status">
  <span class="workspace" title={workspace?.name ?? 'No workspace'}>
    <i
      class="ph {workspace?.location_type === 'LOCAL' ? 'ph-desktop' : 'ph-cloud'}"
      aria-hidden="true"
    ></i>
    <span class="truncate">{workspace?.name ?? 'No workspace'}</span>
  </span>

  <span class="connection" role="status" aria-live="polite">
    <i class="ph {state.icon}" aria-hidden="true"></i>
    <span>{state.label}</span>
  </span>

  {#if persistenceFailed}
    <!-- FR-023: reported, never modal, never blocking the interaction that triggered it. -->
    <span class="warn" role="status"
      ><i class="ph ph-warning" aria-hidden="true"></i> Not saved</span
    >
  {/if}
</footer>

<style>
  .status {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-1) var(--space-3);
    background: var(--color-surface);
    border-top: 1px solid var(--color-divider);
    color: var(--color-neutral-300);
    font-family: var(--font-body);
    /* Fixed height so overlong content cannot displace adjacent content (FR-013). */
    block-size: var(--space-6);
    flex: 0 0 auto;
    overflow: hidden;
  }
  .workspace,
  .connection,
  .warn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-inline-size: 0;
  }
  .workspace {
    /* FR-013: truncate for display; the stored value is untouched. */
    max-inline-size: 40%;
  }
  .truncate {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .connection {
    margin-inline-start: auto;
  }
  .warn {
    color: var(--color-accent-300);
  }
</style>
