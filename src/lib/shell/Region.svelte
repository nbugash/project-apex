<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props {
    label: string;
    visible: boolean;
    extent: number;
    axis: 'inline' | 'block';
    children: Snippet;
  }
  let { label, visible, extent, axis, children }: Props = $props();
</script>

{#if visible}
  <section
    class="region"
    aria-label={label}
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
