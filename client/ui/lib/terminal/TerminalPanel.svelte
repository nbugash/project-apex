<script lang="ts">
  /**
   * One task's terminal, or the idle prompt shown before a task is attached.
   *
   * The two states are genuinely different surfaces rather than one with a flag. Attached, the
   * terminal library owns every pixel inside the host element and this component owns none of
   * them -- including the cursor, which xterm derives from its own cell metrics and cannot be
   * given a size. Idle, there is no library instance at all, and the panel draws the prototype's
   * own accent-coloured prompt: that is what the extracted cursor tokens govern, and it is the
   * only place they apply.
   */
  import type { WorkspaceReference } from '../ipc';
  import { readPalette, watchPalette } from './palette';
  import { terminals } from './terminals.svelte';

  interface Props {
    /** The task whose output this panel shows, or null for the idle prompt. */
    taskId?: string | null;
    workspace: WorkspaceReference | null;
  }
  let { taskId = null, workspace }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);

  /**
   * The prompt the idle panel shows, in the prototype's own two shapes: a host for a remote
   * workspace, a path for a local one. The name is the workspace's, because it is the only name
   * this application actually holds -- inventing a hostname to match the mockup's `build-01`
   * would be putting a fixture on screen and calling it a product.
   */
  const prompt = $derived.by(() => {
    if (!workspace) return '❯';
    return workspace.location_type === 'REMOTE' ? `${workspace.name} ❯` : `~/${workspace.name} ❯`;
  });

  $effect(() => {
    const el = host;
    const id = taskId;
    if (!el || !id) return;
    const panel = terminals.panel(id);
    let stop: (() => void) | null = null;
    // `attach` is async -- the terminal library is imported on demand -- so this effect can be
    // torn down before it resolves. `cancelled` is what stops a panel being mounted into an
    // element that has already left the document.
    let cancelled = false;
    void panel.attach(el, readPalette(el)).then(() => {
      if (cancelled) return;
      stop = watchPalette(el, (theme) => panel.retheme(theme));
    });
    return () => {
      cancelled = true;
      stop?.();
      // Detached, not released: the panel keeps buffering, so hiding this dock tab and showing
      // it again later loses nothing. Releasing is the task set's decision, not the view's.
      panel.detach();
    };
  });
</script>

<div class="transcript" bind:this={host} data-task-id={taskId ?? ''}>
  {#if !taskId}
    <div class="prompt-row">
      <span class="prompt">{prompt}</span>
      <span class="cursor" aria-hidden="true"></span>
    </div>
  {/if}
</div>

<style>
  .transcript {
    flex: 1;
    overflow: auto;
    padding: var(--vk-term-pad);
    font-family: var(--vk-mono);
    font-size: var(--vk-term-fs);
    line-height: var(--vk-term-line-height);
    min-block-size: 0;
  }
  .prompt-row {
    display: flex;
    gap: var(--vk-term-prompt-gap);
  }
  .prompt {
    color: var(--color-accent);
  }
  .cursor {
    inline-size: var(--vk-term-cursor-w);
    block-size: var(--vk-term-cursor-h);
    background: var(--color-accent);
    animation: var(--vk-term-cursor-blink);
  }
</style>
