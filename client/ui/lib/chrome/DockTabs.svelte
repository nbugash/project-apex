<script lang="ts">
  /**
   * The strip below the document area: Terminal, Debug, Problems, Resources.
   *
   * The prototype's own four, in its own order, and that is why this is not a design deviation
   * the way an invented button would be. Three of them belong to features that do not exist, and
   * they are rendered present-and-unavailable for the same reason the activity rail renders its
   * absent destinations: the strip's proportions are part of the prototype, and a strip with one
   * tab in it is a different picture from the one that was designed.
   *
   * Every dimension here comes from `ds-sync`. None of these numbers is this component's to
   * choose — see the `dock tabs` and `dock tab strip` surfaces in `scripts/ds-sync.mjs`.
   */
  interface Props {
    /// Which tab is selected, when the dock is open.
    active: string;
    open: boolean;
    onselect: (id: string) => void;
  }
  let { active, open, onselect }: Props = $props();

  /// The prototype's list. `available` is this application's addition: the prototype has no
  /// notion of a feature that is not built yet, because everything in it is a drawing.
  const TABS = [
    { id: 'terminal', label: 'Terminal', icon: 'ph ph-terminal-window', available: true },
    { id: 'debug', label: 'Debug', icon: 'ph ph-bug', available: false },
    { id: 'problems', label: 'Problems', icon: 'ph ph-warning-circle', available: false },
    { id: 'resources', label: 'Resources', icon: 'ph ph-gauge', available: false },
  ];

  /// Selected *and* open. The prototype highlights on both, so a closed dock shows no tab as
  /// current — which is right: nothing is being shown, so nothing is current.
  const isCurrent = (id: string) => open && active === id;
</script>

<div class="dock-tabs" role="tablist" aria-label="Dock">
  {#each TABS as tab (tab.id)}
    <button
      role="tab"
      class="tab"
      class:current={isCurrent(tab.id)}
      aria-selected={isCurrent(tab.id)}
      aria-disabled={!tab.available}
      tabindex={tab.available ? 0 : -1}
      data-testid={`dock-tab-${tab.id}`}
      onclick={() => tab.available && onselect(tab.id)}
    >
      <i class={tab.icon} aria-hidden="true"></i>{tab.label}
    </button>
  {/each}
</div>

<style>
  .dock-tabs {
    flex: none;
    display: flex;
    align-items: stretch;
    block-size: var(--vk-dock-tabs-height);
    background: var(--color-surface);
    /* The hairline is on the **top** edge here; the editor's tab strip carries its own on the
       bottom. That difference is what tells the two apart in the prototype, and in ds-sync. */
    box-shadow: inset 0 1px 0 color-mix(in srgb, var(--color-text) 9%, transparent);
  }
  .tab {
    display: flex;
    align-items: center;
    gap: var(--vk-dock-tab-gap);
    padding: var(--vk-dock-tab-pad);
    border: 0;
    background: transparent;
    color: color-mix(in srgb, var(--color-text) 55%, transparent);
    font: inherit;
    font-size: var(--vk-dock-tab-fs);
    cursor: pointer;
    position: relative;
  }
  .tab i {
    font-size: var(--vk-dock-tab-icon-fs);
  }
  .tab.current {
    background: var(--color-neutral-900);
    color: var(--color-text);
  }
  .tab:not([aria-disabled='true']):hover {
    background: color-mix(in srgb, var(--color-text) 7%, transparent);
  }
  /* Unavailable tabs are dimmed further and take no pointer, following the activity rail's
     treatment of destinations whose features do not exist. Rendered rather than omitted so the
     strip keeps the prototype's proportions. */
  .tab[aria-disabled='true'] {
    cursor: default;
    opacity: 0.45;
  }
  .tab:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }
</style>
