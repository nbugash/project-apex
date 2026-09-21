<script lang="ts">
  import type { OpenDocumentReference } from '../ipc';
  interface Props {
    documents: OpenDocumentReference[];
    onpick: (id: string) => void;
  }
  let { documents, onpick }: Props = $props();
  let open = $state(false);

  function pick(id: string) {
    open = false;
    onpick(id);
  }
</script>

<!-- FR-006: every tab reachable when more are open than fit. Scrolling alone satisfies the
     letter of that and defeats its purpose at forty tabs; this makes it one interaction. -->
<div class="overflow">
  <button
    class="trigger"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label="All open documents"
    onclick={() => (open = !open)}
  >
    <i class="ph ph-caret-down" aria-hidden="true"></i>
    <span>{documents.length}</span>
  </button>

  {#if open}
    <ul class="list" role="listbox">
      {#each documents as doc (doc.id)}
        <li>
          <button role="option" aria-selected="false" onclick={() => pick(doc.id)}>
            {doc.display_name}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .overflow {
    position: relative;
    flex: 0 0 auto;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    background: transparent;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-sm);
    color: var(--color-neutral-300);
    font-family: var(--font-body);
    cursor: pointer;
  }
  .trigger:hover {
    background: var(--color-neutral-800);
  }
  .trigger:focus-visible,
  .list button:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .list {
    position: absolute;
    inset-block-start: 100%;
    inset-inline-end: 0;
    z-index: 1;
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    background: var(--color-surface);
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    max-block-size: 20rem;
    overflow: auto;
    min-inline-size: 14rem;
  }
  .list button {
    inline-size: 100%;
    text-align: start;
    padding: var(--space-1) var(--space-2);
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font-family: var(--font-body);
    cursor: pointer;
  }
  .list button:hover {
    background: var(--color-neutral-800);
  }
</style>
