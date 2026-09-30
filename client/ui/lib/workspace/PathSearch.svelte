<script lang="ts">
  /// Filter the workspace by path, from what this client holds.
  ///
  /// **This surface is not in the signed-off prototype.** It is recorded as a deviation in spec.md
  /// and plan.md, alongside the conflict panel, and everything here comes from design-system
  /// tokens so a designer can move it without unpicking an improvised value (Principle I).
  ///
  /// The results always come from the projection, online or off: `workspace_search_paths` never
  /// contacts the engine, because the use case behind it holds no provider to fall back to
  /// (FR-031, C6). What changes offline is not where the answer comes from but whether it can be
  /// presented as the whole answer.
  import { invoke } from '@tauri-apps/api/core';

  interface Props {
    /// Called when a result is chosen, with the same signature the tree uses, so choosing a file
    /// here and clicking it there are the same act.
    onOpenFile?: (path: string, name: string) => void;
  }
  let { onOpenFile }: Props = $props();

  interface PathSearch {
    paths: string[];
    complete: boolean;
  }

  let fragment = $state('');
  let result = $state<PathSearch | null>(null);
  let failed = $state(false);

  /// No debounce.
  ///
  /// Measured rather than assumed: path search over 50,000 cached paths has a p99 of about 3 ms
  /// (SC-008), so a keystroke costs an IPC round trip and a millisecond of query. A debounce here
  /// would add latency to every search to save work that is not expensive. If that ever stops
  /// being true, the measurement in `offline_budget.rs` is where it will show.
  async function run(): Promise<void> {
    const asked = fragment.trim();
    if (asked.length === 0) {
      result = null;
      failed = false;
      return;
    }
    try {
      result = await invoke<PathSearch>('workspace_search_paths', { fragment: asked, limit: 50 });
      failed = false;
    } catch {
      // A failed search says nothing about the connection and must not be reported as an outage.
      // The previous results are cleared, because showing them under a new fragment would be
      // worse than showing none.
      result = null;
      failed = true;
    }
  }

  function nameOf(path: string): string {
    return path.slice(path.lastIndexOf('/') + 1);
  }
</script>

<div class="path-search">
  <label class="field">
    <span class="visually-hidden">Filter by path</span>
    <input
      type="text"
      placeholder="Filter by path"
      autocomplete="off"
      spellcheck="false"
      data-testid="path-search-input"
      bind:value={fragment}
      oninput={() => void run()}
    />
  </label>

  {#if failed}
    <p class="note" role="status" data-testid="path-search-failed">Could not search.</p>
  {:else if result}
    {#if !result.complete}
      <!-- FR-007: the results must not be presented as complete. An icon and a word, so the
           caveat is not carried by styling alone, and it sits above the list rather than below it
           because a caveat under a scrolled list is a caveat nobody reads. -->
      <p class="note" role="status" data-testid="path-search-partial">
        <i class="ph ph-hard-drives" aria-hidden="true"></i>
        <span>Showing cached results — this may not be everything</span>
      </p>
    {/if}
    {#if result.paths.length === 0}
      <p class="note" data-testid="path-search-empty">Nothing cached matches.</p>
    {:else}
      <ul class="results" data-testid="path-search-results">
        {#each result.paths as path (path)}
          <li>
            <button type="button" data-path={path} onclick={() => onOpenFile?.(path, nameOf(path))}>
              <span class="name">{nameOf(path)}</span>
              <span class="dir">{path}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .path-search {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    min-inline-size: 0;
  }
  .field {
    display: flex;
    min-inline-size: 0;
  }
  input {
    inline-size: 100%;
    min-inline-size: 0;
    padding: var(--space-1) var(--space-2);
    font: inherit;
    font-family: var(--font-body);
    color: var(--color-neutral-100);
    background: var(--color-bg);
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-sm);
  }
  .note {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    /* The neutral ramp: an incomplete answer while offline is an ordinary consequence of being
       offline, not a fault. */
    color: var(--color-neutral-400);
  }
  .results {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow: auto;
    min-block-size: 0;
  }
  .results button {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    inline-size: 100%;
    padding: var(--space-1) var(--space-2);
    font: inherit;
    text-align: start;
    color: inherit;
    background: none;
    border: 0;
    cursor: pointer;
  }
  .results button:hover,
  .results button:focus-visible {
    background: var(--color-neutral-800);
  }
  .name {
    color: var(--color-neutral-100);
  }
  .dir {
    color: var(--color-neutral-500);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .visually-hidden {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
