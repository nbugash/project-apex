<script lang="ts">
  import RailButton from './RailButton.svelte';
  import { inRailOrder, nextSelectable } from '../rail';
  import type { RailDestination } from '../ipc';

  interface Props {
    destinations: RailDestination[];
    activeId: string | null;
    collapsed: boolean;
    onselect: (id: string) => void;
    ontoggle: () => void;
  }

  let { destinations, activeId, collapsed, onselect, ontoggle }: Props = $props();

  let ordered = $derived(inRailOrder(destinations));

  // Roving tab index (FR-007): the rail is one tab stop, and arrow keys move within it.
  // Without this every unavailable destination would still take a Tab press, so reaching
  // the editor from the header would cost seven of them.
  let focusedId = $state<string | null>(null);
  let rail = $state<HTMLElement | null>(null);

  let tabStopId = $derived(
    focusedId ?? activeId ?? ordered.find((d) => d.available)?.id ?? ordered[0]?.id ?? null,
  );

  function moveFocus(step: 1 | -1) {
    const target = nextSelectable(ordered, tabStopId, step);
    if (!target) return;
    focusedId = target.id;
    // The wrapper is `display: contents` and not focusable; the button inside it is.
    rail?.querySelector<HTMLElement>(`[data-destination="${target.id}"] button`)?.focus();
  }

  function onkeydown(event: KeyboardEvent) {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        moveFocus(1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        moveFocus(-1);
        break;
      default:
    }
  }
</script>

<nav
  class="rail"
  aria-label="Tool windows"
  role="tablist"
  aria-orientation="vertical"
  bind:this={rail}
  {onkeydown}
>
  {#each ordered as destination (destination.id)}
    <div class="slot" data-destination={destination.id}>
      <RailButton
        {destination}
        active={destination.id === activeId}
        tabindex={destination.id === tabStopId ? 0 : -1}
        {onselect}
      />
    </div>
  {/each}

  <span class="spacer"></span>

  <button
    class="toggle"
    type="button"
    title={collapsed ? 'Show tool window' : 'Collapse tool window'}
    aria-label={collapsed ? 'Show tool window' : 'Collapse tool window'}
    aria-expanded={!collapsed}
    onclick={ontoggle}
  >
    <i class="ph ph-sidebar-simple" aria-hidden="true"></i>
  </button>
</nav>

<style>
  .rail {
    flex: 0 0 auto;
    inline-size: var(--vk-rail-width);
    background: var(--color-surface);
    box-shadow: inset -1px 0 0 color-mix(in srgb, var(--color-text) 9%, transparent);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: var(--vk-rail-pad);
    gap: var(--vk-rail-gap);
  }
  /* The wrapper exists only to carry the destination identifier for focus management;
     giving it no box of its own keeps the rail's spacing exactly the prototype's. */
  .slot {
    display: contents;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .toggle {
    inline-size: var(--vk-rail-toggle);
    block-size: var(--vk-rail-toggle);
    display: grid;
    place-items: center;
    border: 0;
    background: transparent;
    color: var(--color-neutral-500);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .toggle:hover {
    color: var(--color-text);
  }
  /* One pixel smaller than a destination icon in the prototype. Reusing the destination
     size here would be an invisible-looking change that the fidelity gate would catch
     later and more expensively. */
  .toggle i {
    font-size: var(--vk-rail-toggle-icon);
  }
  .toggle:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
</style>
