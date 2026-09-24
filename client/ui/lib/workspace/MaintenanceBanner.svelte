<script lang="ts">
  /// Startup maintenance, while it runs.
  ///
  /// Shown for the whole duration and refreshed as the phase changes (FR-018a): an upgrade that
  /// appears to hang is indistinguishable from a broken install. Migrating stays distinct from
  /// evicting, because SC-013a asserts on migration reports specifically.
  import { presentMaintenance } from '../statusbar/presentation';

  let { phase, progress }: { phase: string; progress?: { from: number; to: number } } = $props();

  const presented = $derived(presentMaintenance(phase, progress));
</script>

{#if presented}
  <div class="maintenance-banner" role="status" data-testid="maintenance-banner" data-phase={phase}>
    <i class="ph {presented.icon}" aria-hidden="true"></i>
    <span class="label">{presented.label}</span>
  </div>
{/if}

<style>
  .maintenance-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--color-surface);
    color: var(--color-text);
    font-family: var(--font-body);
    font-size: var(--vk-status-size);
  }
</style>
