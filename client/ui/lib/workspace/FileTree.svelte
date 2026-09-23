<script lang="ts">
  /// The file tree.
  ///
  /// Tokens and Phosphor glyphs only — no raw hex, no raw pixel values, no font families
  /// (Principle I). Keyboard-operable: focusable, showing the design system's `:focus-visible`
  /// ring rather than the browser default, and expanding from the keyboard (FR-040). Arrow-key
  /// traversal and type-ahead are F006's (FR-040a) and deliberately absent.
  import type { WorkspaceTree } from './tree.svelte';
  import { presentContent } from '../statusbar/presentation';

  let { tree }: { tree: WorkspaceTree } = $props();

  function onKey(event: KeyboardEvent, path: string) {
    // Enter and Space are the two keys that activate a control everywhere else in the system.
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      void tree.toggle(path);
    }
  }
</script>

<div class="file-tree" role="tree" aria-label="Workspace files" data-testid="file-tree">
  {#if tree.problem}
    <p class="problem" data-testid="tree-problem">
      <i
        class="ph {presentContent(tree.problem.kind === 'gone' ? 'gone' : 'unavailable').icon}"
        aria-hidden="true"
      ></i>
      <span>
        {tree.problem.kind === 'gone'
          ? presentContent('gone').label
          : presentContent('unavailable').label}
      </span>
    </p>
  {/if}

  {#each tree.nodes as node (node.path)}
    <div
      class="row"
      role="treeitem"
      tabindex="0"
      aria-expanded={node.kind === 'directory' ? node.expanded : undefined}
      aria-level={node.depth + 1}
      data-testid="tree-row"
      data-path={node.path}
      style={`--depth: ${node.depth}`}
      onclick={() => tree.toggle(node.path)}
      onkeydown={(e) => onKey(e, node.path)}
    >
      <i
        class="ph {node.kind === 'directory'
          ? node.expanded
            ? 'ph-caret-down'
            : 'ph-caret-right'
          : 'ph-file'}"
        aria-hidden="true"
      ></i>
      <span class="name">{node.name}</span>
    </div>
  {/each}
</div>

<style>
  .file-tree {
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    font-family: var(--font-body);
    font-size: var(--vk-status-size);
    color: var(--color-text);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    /* The indent is the only place depth appears, so a deep tree cannot drift out of alignment
       with a shallow one. */
    padding-inline-start: calc(var(--space-2) + var(--space-3) * var(--depth));
    padding-block: var(--space-1);
    cursor: default;
    user-select: none;
  }

  .row:hover {
    background: var(--color-surface);
  }

  /* The design system's ring, never the browser default (Principle I, FR-040). */
  .row:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .problem {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3);
    color: var(--color-neutral-500);
  }
</style>
