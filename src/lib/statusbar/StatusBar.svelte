<script lang="ts">
  import type { ConnectionState, WorkspaceReference } from '../ipc';
  import { present } from './presentation';

  interface Props {
    connection: ConnectionState;
    workspace: WorkspaceReference | null;
    persistenceFailed?: boolean;
  }
  let { connection, workspace, persistenceFailed = false }: Props = $props();

  let state = $derived(present(connection));
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
    /* The prototype's own metrics, extracted by ds:sync. F000 built this bar from the
       generic spacing scale, which put it at 17px against the prototype's 26px — a
       difference nobody spotted by eye, but one that shortened the activity rail and the
       tool window by nine pixels each and so failed the fidelity gate. */
    gap: var(--vk-status-gap);
    padding: var(--vk-status-pad);
    background: var(--color-surface);
    box-shadow: inset 0 1px 0 color-mix(in srgb, var(--color-text) 9%, transparent);
    color: color-mix(in srgb, var(--color-text) 58%, transparent);
    font-family: var(--font-body);
    font-size: var(--vk-status-size);
    /* Fixed height so overlong content cannot displace adjacent content (FR-013). */
    block-size: var(--vk-status-height);
    white-space: nowrap;
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
