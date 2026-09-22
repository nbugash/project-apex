<script lang="ts">
  interface Props {
    /** The open workspace's name, or null when none is open. */
    workspace: string | null;
  }

  let { workspace }: Props = $props();

  // The prototype shows a project name because it is a populated mock. With no workspace
  // open the switcher still renders — removing it would change the header's geometry —
  // and says so, matching how the status bar already reports the same absence.
  let projectName = $derived(workspace ?? 'No workspace');
</script>

<!--
  Every control here belongs to a feature that does not exist yet: opening a workspace,
  run configurations, the omnibox, remote targets, settings. They render at full fidelity
  and are marked `aria-disabled` with no tab stop, so the header looks exactly like the
  approved design without claiming behaviour it does not have. The alternative — omitting
  them until their features land — would change the header's proportions on every release
  and make the one surface users see first the least stable thing in the application.
-->
<header class="chrome" aria-label="Application chrome">
  <div class="brand">
    <span class="mark" aria-hidden="true"></span>
    <span class="wordmark">Apex</span>
  </div>

  <button class="switcher" type="button" aria-disabled="true" tabindex="-1">
    <i class="ph ph-cube" aria-hidden="true"></i>
    <span>{projectName}</span>
    <i class="ph ph-caret-down caret" aria-hidden="true"></i>
  </button>

  <span class="divider" aria-hidden="true"></span>

  <div class="run-group">
    <button class="run-config" type="button" aria-disabled="true" tabindex="-1">
      <i class="ph ph-play-circle run-config-icon" aria-hidden="true"></i>
      <span>No run configuration</span>
      <i class="ph ph-caret-down run-config-caret" aria-hidden="true"></i>
    </button>
    <button class="icon-button" type="button" aria-label="Run" aria-disabled="true" tabindex="-1">
      <i class="ph ph-play run-icon" aria-hidden="true"></i>
    </button>
    <button class="icon-button" type="button" aria-label="Debug" aria-disabled="true" tabindex="-1">
      <i class="ph ph-bug debug-icon" aria-hidden="true"></i>
    </button>
  </div>

  <button class="omnibox" type="button" aria-disabled="true" tabindex="-1">
    <i class="ph ph-magnifying-glass omni-icon" aria-hidden="true"></i>
    <span class="omni-label">Search everywhere</span>
    <span class="kbd">⌘K</span>
  </button>

  <button class="pill" type="button" aria-disabled="true" tabindex="-1">
    <i class="ph ph-crosshair pill-icon" aria-hidden="true"></i>
    <span>Spec pins</span>
  </button>

  <button class="pill" type="button" aria-disabled="true" tabindex="-1">
    <i class="ph ph-desktop pill-icon" aria-hidden="true"></i>
    <span>Local</span>
  </button>

  <button
    class="icon-button"
    type="button"
    aria-label="Settings"
    aria-disabled="true"
    tabindex="-1"
  >
    <i class="ph ph-sliders-horizontal settings-icon" aria-hidden="true"></i>
  </button>
</header>

<style>
  .chrome {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--vk-chrome-gap);
    block-size: var(--vk-chrome-height);
    padding: var(--vk-chrome-pad);
    white-space: nowrap;
    background: var(--color-surface);
    box-shadow: inset 0 -1px 0 color-mix(in srgb, var(--color-text) 9%, transparent);
    position: relative;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--vk-chrome-mark-gap);
  }
  .mark {
    inline-size: var(--vk-chrome-mark);
    block-size: var(--vk-chrome-mark);
    border-radius: 2px;
    background: var(--color-accent);
    box-shadow: var(--vk-chrome-mark-glow);
  }
  .wordmark {
    font-family: var(--font-heading);
    font-weight: var(--font-heading-weight);
    font-size: var(--vk-chrome-wordmark-size);
    letter-spacing: var(--vk-chrome-wordmark-tracking);
    text-transform: uppercase;
  }

  .switcher {
    display: flex;
    align-items: center;
    gap: var(--vk-chrome-switcher-gap);
    border: 0;
    background: transparent;
    color: var(--color-text);
    font: inherit;
    cursor: pointer;
    padding: var(--vk-chrome-switcher-pad);
    border-radius: var(--radius-sm);
  }
  .switcher i {
    font-size: var(--vk-chrome-switcher-icon);
    color: var(--color-neutral-400);
  }
  .switcher .caret {
    font-size: var(--vk-chrome-caret);
    color: var(--color-neutral-500);
  }

  .divider {
    inline-size: 1px;
    block-size: var(--vk-chrome-divider-height);
    background: color-mix(in srgb, var(--color-text) 12%, transparent);
  }

  .run-group {
    display: flex;
    align-items: center;
    gap: var(--vk-chrome-run-gap);
  }
  .run-config {
    display: flex;
    align-items: center;
    gap: var(--vk-chrome-runcfg-gap);
    border: 1px solid color-mix(in srgb, var(--color-text) 12%, transparent);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--vk-chrome-runcfg-size);
    cursor: pointer;
    padding: var(--vk-chrome-switcher-pad);
    border-radius: var(--radius-sm);
  }
  .run-config-icon {
    font-size: var(--vk-chrome-runcfg-icon);
    color: var(--color-accent);
  }
  .run-config-caret {
    font-size: var(--vk-chrome-runcfg-caret);
    color: var(--color-neutral-500);
  }

  .icon-button {
    inline-size: var(--vk-chrome-iconbutton);
    block-size: var(--vk-chrome-iconbutton);
    display: grid;
    place-items: center;
    border: 0;
    background: transparent;
    color: var(--color-neutral-300);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .run-icon {
    font-size: var(--vk-chrome-run-icon);
  }
  .debug-icon,
  .settings-icon {
    font-size: var(--vk-chrome-debug-icon);
  }

  .omnibox {
    flex: 1 1 0;
    max-inline-size: var(--vk-chrome-omni-max);
    margin: 0 auto;
    display: flex;
    align-items: center;
    gap: var(--vk-chrome-omni-gap);
    block-size: var(--vk-chrome-omni-height);
    padding: var(--vk-chrome-omni-pad);
    background: var(--color-bg);
    border: 1px solid color-mix(in srgb, var(--color-text) 10%, transparent);
    border-radius: var(--radius-md);
    color: color-mix(in srgb, var(--color-text) 50%, transparent);
    font: inherit;
    font-size: var(--vk-chrome-omni-size);
    cursor: pointer;
    text-align: left;
  }
  .omni-icon {
    flex: 0 0 auto;
    font-size: var(--vk-chrome-omni-icon);
  }
  .omni-label {
    flex: 1 1 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kbd {
    /* The design system defines no monospace token, yet the prototype uses one here.
       That gap is FR-022's case: rather than improvise a stack, the prototype's own is
       extracted alongside the dimensions. */
    font-family: var(--vk-mono);
    font-size: var(--vk-chrome-kbd-size);
    padding: var(--vk-chrome-kbd-pad);
    border-radius: var(--vk-tool-button-radius);
    background: color-mix(in srgb, var(--color-text) 8%, transparent);
  }

  .pill {
    display: flex;
    align-items: center;
    gap: var(--vk-chrome-pill-gap);
    border: 1px solid color-mix(in srgb, var(--color-text) 12%, transparent);
    background: transparent;
    color: color-mix(in srgb, var(--color-text) 55%, transparent);
    font: inherit;
    font-size: var(--vk-chrome-pill-size);
    cursor: pointer;
    padding: var(--vk-chrome-pill-pad);
    border-radius: var(--radius-sm);
  }
  .pill-icon {
    font-size: var(--vk-chrome-pill-icon);
  }
</style>
