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
    /// What to show on the Terminal tab, or null while nothing has ended.
    ///
    /// The prototype's tabs carry a badge already -- the `1` on Problems -- so this uses the
    /// design's own affordance rather than adding one.
    terminalBadge?: { badge: string; spoken: string; ok: boolean } | null;
  }
  let { active, open, onselect, terminalBadge = null }: Props = $props();

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

  const badgeFor = (id: string) => (id === 'terminal' ? terminalBadge : null);

  /// What a screen reader hears. The badge is text and survives greyscale, but `1` is not a
  /// sentence, so the tab says the whole thing (FR-029).
  function nameFor(tab: { id: string; label: string; available: boolean }): string {
    const badge = badgeFor(tab.id);
    if (badge) return `${tab.label} — ${badge.spoken}`;
    return tab.available ? tab.label : `${tab.label} — not available yet`;
  }
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
      aria-label={nameFor(tab)}
      title={nameFor(tab)}
    >
      <i class={tab.icon} aria-hidden="true"></i>{tab.label}
      {#if badgeFor(tab.id)}
        {@const badge = badgeFor(tab.id)!}
        <!-- The mark and the text are both non-colour channels: a check against a cross is a
             difference in shape, and `0` against `1` a difference in glyph. The hue is the third
             channel and never the only one (FR-029). `aria-hidden` because the button's own
             accessible name already says all of this in words. -->
        <span class="badge" class:ok={badge.ok} class:bad={!badge.ok} aria-hidden="true">
          <i class={badge.ok ? 'ph ph-check' : 'ph ph-x'}></i>{badge.badge}
        </span>
      {/if}
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
  .badge {
    display: inline-flex;
    align-items: center;
    gap: calc(var(--vk-dock-tab-gap) / 2);
    font-size: var(--vk-dock-tab-badge-fs);
  }
  .badge i {
    font-size: var(--vk-dock-tab-badge-fs);
  }
  /* The design system's own two hues, and the third channel rather than the only one. */
  .badge.ok {
    color: var(--vk-term-ansi-green);
  }
  .badge.bad {
    color: var(--vk-term-ansi-red);
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
