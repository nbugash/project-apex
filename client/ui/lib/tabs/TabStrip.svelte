<script lang="ts">
  import type { OpenDocumentReference } from '../ipc';
  import TabOverflow from './TabOverflow.svelte';
  import { inOrder, neighbour } from './ordering';

  interface Props {
    documents: OpenDocumentReference[];
    focusedId: string | null;
    onfocus: (id: string) => void;
    onclose: (id: string) => void;
    onreorder: (id: string, toOrder: number) => void;
    /// Documents whose file changed on the host since it was read (FR-023, FR-023a).
    ///
    /// Marked on the tab it concerns rather than by taking focus: reporting a background tab
    /// by pulling the developer to it would be the interruption FR-024 forbids, arriving
    /// through the requirement meant to inform them.
    ///
    /// Rendered as **dimming**, the same treatment the tree gives a stale region, because it
    /// is the same fact: what you are being shown may have moved on. Not a marker — the
    /// change is almost always something the developer did seconds ago in the shell beside
    /// this window, so they need the interface to stop implying currency, not to be alerted
    /// about their own `git checkout`.
    changedOnHost?: string[];
  }
  let {
    documents,
    focusedId,
    onfocus,
    onclose,
    onreorder,
    changedOnHost = [],
  }: Props = $props();

  let strip: HTMLElement | undefined = $state();
  let draggingId: string | null = $state(null);

  let ordered = $derived(inOrder(documents));

  export function scrollIntoView(id: string) {
    strip?.querySelector<HTMLElement>(`[data-tab="${id}"]`)?.scrollIntoView({
      block: 'nearest',
      inline: 'nearest',
    });
  }

  function pickFromOverflow(id: string) {
    onfocus(id);
    queueMicrotask(() => scrollIntoView(id));
  }

  // FR-018: tabs navigable without a pointer.
  function key(event: KeyboardEvent, id: string) {
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (step === 0) return;
    const next = neighbour(documents, id, step);
    if (next) {
      onfocus(next.id);
      queueMicrotask(() => scrollIntoView(next.id));
    }
    event.preventDefault();
  }

  function drop(event: DragEvent, toOrder: number) {
    event.preventDefault();
    if (draggingId) onreorder(draggingId, toOrder);
    draggingId = null;
  }
</script>

<div class="tabbar">
  <div class="strip" bind:this={strip} role="tablist" aria-label="Open documents">
    {#each ordered as doc, i (doc.id)}
      <div
        class="tab"
        class:active={doc.id === focusedId}
        class:stale={changedOnHost.includes(doc.id)}
        data-tab={doc.id}
        data-testid={changedOnHost.includes(doc.id) ? 'tab-changed' : undefined}
        title={changedOnHost.includes(doc.id)
          ? 'Changed on the host since it was read'
          : undefined}
        aria-description={changedOnHost.includes(doc.id)
          ? 'Changed on the host since it was read'
          : undefined}
        role="tab"
        tabindex={doc.id === focusedId ? 0 : -1}
        aria-selected={doc.id === focusedId}
        draggable="true"
        ondragstart={() => (draggingId = doc.id)}
        ondragover={(e) => e.preventDefault()}
        ondrop={(e) => drop(e, i)}
        onclick={() => onfocus(doc.id)}
        onkeydown={(e) => key(e, doc.id)}
      >
        <span class="label">{doc.display_name}</span>

        <button
          class="close"
          aria-label={`Close ${doc.display_name}`}
          onclick={(e) => {
            e.stopPropagation();
            onclose(doc.id);
          }}
        >
          <i class="ph ph-x" aria-hidden="true"></i>
        </button>
      </div>
    {/each}
  </div>

  {#if ordered.length > 0}
    <TabOverflow documents={ordered} onpick={pickFromOverflow} />
  {/if}
</div>

<style>
  /* The same dimming the tree gives a stale row — "dimmed rows are stale, they are never
     waited on". One vocabulary for "this may have moved on", on both surfaces. It composes
     with the dirty dot rather than competing with it: a tab can be dimmed *and* carry the
     dot, which is the case where a developer most needs both facts, and they arrive on
     different channels. The title and aria-description are what carry the meaning when
     colour and contrast are gone (FR-039). */
  .tab.stale .label {
    opacity: 0.62;
  }

  .tabbar {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding-inline: var(--space-1);
    background: var(--color-surface);
    border-block-end: 1px solid var(--color-divider);
    flex: 0 0 auto;
    min-inline-size: 0;
  }
  .strip {
    display: flex;
    gap: var(--space-1);
    overflow-x: auto;
    scrollbar-width: thin;
    flex: 1 1 auto;
    min-inline-size: 0;
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    color: var(--color-neutral-300);
    font-family: var(--font-body);
    white-space: nowrap;
    cursor: pointer;
    /* Labels never shrink to illegibility: the strip scrolls instead. */
    flex: 0 0 auto;
  }
  .tab:hover {
    background: var(--color-neutral-800);
  }
  .tab.active {
    background: var(--color-bg);
    color: var(--color-text);
    box-shadow: inset 0 -2px 0 0 var(--color-accent);
  }
  .tab:focus-visible,
  .close:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .close {
    background: transparent;
    border: 0;
    padding: 0;
    color: inherit;
    cursor: pointer;
    display: inline-flex;
  }
</style>
