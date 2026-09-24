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

  let {
    tree,
    selected = '',
    onWatchedChanged,
  }: {
    tree: WorkspaceTree;
    selected?: string;
    /// Called with every expanded folder whenever expansion changes. The parent combines it
    /// with the open tabs and asks the engine; this component does not know the protocol.
    onWatchedChanged?: (expanded: string[]) => void;
  } = $props();

  /// The prototype's glyph per kind. Directories carry a caret so expansion state is legible
  /// without colour, which is also what FR-039 requires of every state this feature publishes.
  function glyph(kind: string, expanded: boolean): string {
    if (kind !== 'directory') return 'ph-file-code';
    return expanded ? 'ph-folder-open' : 'ph-folder';
  }

  function onKey(event: KeyboardEvent, path: string) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      void toggle(path);
    }
  }

  /// Expanding starts watching and collapsing releases it; freshness follows attention
  /// (FR-003a). The set is declared after the toggle rather than computed from it, so the
  /// request is a statement of what is open rather than a diff somebody has to keep correct.
  async function toggle(path: string) {
    await tree.toggle(path);
    onWatchedChanged?.(tree.expandedFolders());
  }
</script>

<div
  class="file-tree"
  class:stale={tree.stale}
  role="tree"
  aria-label="Project files"
  data-testid="file-tree"
  data-stale={tree.stale ? 'true' : 'false'}
>
  {#if tree.stale}
    <!-- The prototype's own treatment: "Dimmed rows are stale … They are never waited on."
         Dimming follows it rather than inventing a state (Principle I). The text is what makes
         the meaning reachable without colour, which FR-039 requires and `lint:ds` cannot see. -->
    <p class="stale-note" data-testid="tree-stale">
      <i class="ph ph-clock-countdown" aria-hidden="true"></i>
      <span>Showing what was last read — reopening a folder refreshes it</span>
    </p>
  {/if}
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
      onclick={() => toggle(node.path)}
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

  .stale-note {
    display: flex;
    align-items: center;
    gap: var(--vk-tree-gap);
    padding: var(--space-1) var(--vk-tree-pad-left);
    color: var(--color-text);
    font-size: var(--vk-fs);
    /* Dimmed by opacity rather than a colour token. The prototype expresses staleness that
       way -- "dimmed rows are stale" -- and inventing a colour for it would be a deviation
       needing designer approval rather than a reading of what is already there. */
    opacity: 0.72;
  }

  /* Dimmed, following the prototype. Not hidden and not emptied: a tree that vanished on a
     branch switch would be worse than one saying it may have moved on. */
  .file-tree.stale .row {
    opacity: 0.62;
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
