<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** The active destination's name, shown as the panel's title (FR-003). */
    title: string;
    /** Secondary text on the right of the header — a file count in the prototype. */
    meta?: string;
    width: number;
    collapsed: boolean;
    ontoggle: () => void;
    children?: Snippet;
  }

  let { title, meta = '', width, collapsed, ontoggle, children }: Props = $props();
</script>

{#if !collapsed}
  <aside class="tool-window" style="inline-size: {width}px" aria-label={title}>
    <div class="header">
      <span class="label">{title}</span>
      <span class="spacer"></span>
      {#if meta}<span class="meta">{meta}</span>{/if}
      <button
        class="collapse"
        type="button"
        title="Collapse panel"
        aria-label="Collapse panel"
        onclick={ontoggle}
      >
        <i class="ph ph-caret-double-left" aria-hidden="true"></i>
      </button>
    </div>
    <div class="body">
      {@render children?.()}
    </div>
  </aside>
{/if}

<style>
  /* The width is inline because it is state, not design: the user drags it and the core
     persists it. Every other dimension here comes from the prototype's tokens. */
  .tool-window {
    flex: 0 0 auto;
    min-inline-size: 0;
    background: var(--color-neutral-900);
    box-shadow: inset -1px 0 0 color-mix(in srgb, var(--color-text) 9%, transparent);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    position: relative;
  }
  .header {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--vk-tool-header-gap);
    block-size: var(--vk-tool-header-height);
    padding: var(--vk-tool-header-pad);
  }
  .label {
    font-size: var(--vk-tool-label-size);
    letter-spacing: var(--vk-tool-label-tracking);
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-text) 55%, transparent);
  }
  .spacer {
    flex: 1 1 auto;
  }
  .meta {
    font-size: var(--vk-tool-meta-size);
    color: color-mix(in srgb, var(--color-text) 52%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .collapse {
    flex: 0 0 auto;
    inline-size: var(--vk-tool-button);
    block-size: var(--vk-tool-button);
    display: grid;
    place-items: center;
    border: 0;
    background: transparent;
    color: var(--color-neutral-500);
    cursor: pointer;
    /* The prototype writes a literal here rather than reaching for --radius-sm, which is
       4px. Extracting its value keeps the one-pixel difference the designer chose. */
    border-radius: var(--vk-tool-button-radius);
  }
  .collapse:hover {
    background: color-mix(in srgb, var(--color-text) 8%, transparent);
    color: var(--color-text);
  }
  .collapse i {
    font-size: var(--vk-tool-button-icon);
  }
  .collapse:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .body {
    flex: 1 1 auto;
    min-block-size: 0;
    overflow: auto;
  }
</style>
