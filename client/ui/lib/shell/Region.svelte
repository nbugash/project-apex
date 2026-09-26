<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props {
    label: string;
    visible: boolean;
    extent: number;
    axis: 'inline' | 'block';
    /** A stable hook for the end-to-end suite.
     *
     * Separate from `label`, which is the accessible name and is allowed to change with the
     * mode -- the dock reads "Terminal — <workspace>" remotely and "Terminal — local" locally.
     * A suite selecting on that string breaks the first time a name is improved, and it broke
     * exactly once before this existed. */
    testid?: string;
    children: Snippet;
  }
  let { label, visible, extent, axis, testid, children }: Props = $props();
</script>

{#if visible}
  <section
    class="region"
    aria-label={label}
    data-testid={testid}
    style={axis === 'inline' ? `inline-size:${extent}px` : `block-size:${extent}px`}
  >
    {@render children()}
  </section>
{/if}

<style>
  .region {
    background: var(--color-surface);
    color: var(--color-text);
    font-family: var(--font-body);
    overflow: auto;
    flex: 0 0 auto;
    min-inline-size: 0;
    min-block-size: 0;
  }
</style>
