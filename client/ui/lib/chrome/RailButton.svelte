<script lang="ts">
  import type { RailDestination } from '../ipc';

  interface Props {
    destination: RailDestination;
    active: boolean;
    /** Roving tab index: exactly one button in the rail is in the tab order (FR-007). */
    tabindex: number;
    onselect: (id: string) => void;
  }

  let { destination, active, tabindex, onselect }: Props = $props();
</script>

<!--
  An unavailable destination is rendered, not omitted: removing it would change the rail's
  height and therefore its fidelity (FR-008). It is marked `aria-disabled` rather than
  `disabled` so its name is still announced — a user asking "what will this application do
  eventually" gets an answer, and a destination that silently vanishes until some later
  release is worse than one that says it is not ready.
-->
<button
  class="rail-button"
  class:active
  class:unavailable={!destination.available}
  type="button"
  role="tab"
  aria-selected={active}
  aria-disabled={!destination.available}
  title={destination.label}
  aria-label={destination.label}
  {tabindex}
  onclick={() => destination.available && onselect(destination.id)}
>
  <i class="ph {destination.icon}" aria-hidden="true"></i>
  <span class="mark" aria-hidden="true"></span>
</button>

<style>
  .rail-button {
    position: relative;
    inline-size: var(--vk-rail-button);
    block-size: var(--vk-rail-button);
    display: grid;
    place-items: center;
    border: 0;
    background: transparent;
    color: var(--color-neutral-500);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .rail-button:hover {
    background: color-mix(in srgb, var(--color-text) 8%, transparent);
  }
  .rail-button.active {
    background: color-mix(in srgb, var(--color-text) 8%, transparent);
    color: var(--color-text);
  }

  /* The prototype defines no unavailable state — every destination in it works. Reduced
     opacity is this feature's answer to FR-008 and is recorded as a gap for the designer
     (FR-022). It is deliberately not a colour change: opacity survives greyscale, so the
     three states stay distinguishable without relying on hue (SC-004). */
  .rail-button.unavailable {
    opacity: 0.45;
    cursor: default;
  }
  .rail-button.unavailable:hover {
    background: transparent;
  }

  .rail-button i {
    font-size: var(--vk-rail-icon);
  }

  /* The active mark sits outside the button box, against the rail's edge. */
  .mark {
    position: absolute;
    left: var(--vk-rail-mark-left);
    top: var(--vk-rail-mark-top);
    inline-size: 2px;
    block-size: var(--vk-rail-mark-height);
    border-radius: 1px;
    background: transparent;
  }
  .rail-button.active .mark {
    background: var(--color-accent);
  }

  .rail-button:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
</style>
