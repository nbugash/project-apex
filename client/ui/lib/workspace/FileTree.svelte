<script lang="ts">
  /// The project panel's file tree.
  ///
  /// Every dimension comes from `--vk-tree-*`, which `scripts/ds-sync.mjs` extracts from the
  /// signed-off prototype's own tree rows (Principle I). Nothing here is improvised from an
  /// adjacent value: the prototype owns these numbers, and a literal in this file would be a
  /// second source of truth for them.
  ///
  /// Keyboard-operable, with the design system's focus ring rather than the browser default
  /// (FR-040). Arrow-key traversal and type-ahead belong to F006 (FR-040a) and are deliberately
  /// absent.
  import type { WorkspaceTree } from './tree.svelte';
  import { presentContent } from '../statusbar/presentation';

  let { tree, selected = '' }: { tree: WorkspaceTree; selected?: string } = $props();

  /// The prototype's glyph per kind. Directories carry a caret so expansion state is legible
  /// without colour, which is also what FR-039 requires of every state this feature publishes.
  function glyph(kind: string, expanded: boolean): string {
    if (kind !== 'directory') return 'ph-file-code';
    return expanded ? 'ph-folder-open' : 'ph-folder';
  }

  function onKey(event: KeyboardEvent, path: string) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      void tree.toggle(path);
    }
  }
</script>

<div class="file-tree" role="tree" aria-label="Project files" data-testid="file-tree">
  {#if tree.problem}
    <p class="problem" data-testid="tree-problem" data-kind={tree.problem.kind}>
      <i
        class="ph {presentContent(tree.problem.kind === 'gone' ? 'gone' : 'unavailable').icon}"
        aria-hidden="true"
      ></i>
      <span>{presentContent(tree.problem.kind === 'gone' ? 'gone' : 'unavailable').label}</span>
    </p>
  {/if}

  {#each tree.nodes as node (node.path)}
    <div
      class="row"
      class:selected={node.path === selected}
      role="treeitem"
      tabindex="0"
      aria-expanded={node.kind === 'directory' ? node.expanded : undefined}
      aria-level={node.depth + 1}
      aria-selected={node.path === selected}
      data-testid="tree-row"
      data-path={node.path}
      style={`padding-left: calc(var(--vk-tree-pad-left) + var(--vk-tree-indent) * ${node.depth})`}
      onclick={() => tree.toggle(node.path)}
      onkeydown={(e) => onKey(e, node.path)}
    >
      <i class="ph {glyph(node.kind, node.expanded)}" aria-hidden="true"></i>
      <span class="name">{node.name}</span>
      <!-- The prototype's VCS marker column. F011 fills it; the column exists here so adding it
           later does not reflow every row. -->
      <span class="vcs" aria-hidden="true"></span>
    </div>
  {/each}
</div>

<style>
  .file-tree {
    display: flex;
    flex-direction: column;
    overflow: auto;
    font-family: var(--font-body);
    font-size: var(--vk-fs);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--vk-tree-gap);
    height: var(--vk-row);
    padding-right: var(--vk-tree-pad-right);
    background: transparent;
    color: var(--color-neutral-300);
    cursor: pointer;
    white-space: nowrap;
  }

  .row:hover {
    background: color-mix(in srgb, var(--color-text) 6%, transparent);
  }

  .row.selected {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    color: var(--color-text);
  }

  /* The design system's ring, never the browser default (Principle I, FR-040). */
  .row:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }

  .row i {
    flex: none;
    font-size: var(--vk-tree-icon);
    color: var(--color-neutral-500);
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .vcs {
    flex: none;
    font-family: var(--vk-mono);
    font-size: var(--vk-tree-vcs-size);
  }

  .problem {
    display: flex;
    align-items: center;
    gap: var(--vk-tree-gap);
    padding: var(--space-3);
    color: var(--color-neutral-500);
  }
</style>
