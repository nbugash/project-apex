<script lang="ts">
  /// What the developer is told about the content they are about to see.
  ///
  /// Every state carries a glyph **and** a label, so none is conveyed by colour alone and all
  /// survive a greyscale render (FR-039, SC-016).
  import { presentContent } from '../statusbar/presentation';

  let { state }: { state: string } = $props();
  const presented = $derived(presentContent(state));
</script>

<span class="verify-badge" data-testid="verify-badge" data-state={state}>
  <i class="ph {presented.icon}" aria-hidden="true"></i>
  <span class="label">{presented.label}</span>
</span>

<style>
  .verify-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-family: var(--font-body);
    font-size: var(--vk-status-size);
    color: var(--color-neutral-500);
  }
</style>
