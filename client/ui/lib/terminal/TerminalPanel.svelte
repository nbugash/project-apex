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
  import { taskSink } from './sink';
  import { inputBytes } from './wire';
  import { terminals } from './terminals.svelte';

  /// What a bare "start a terminal" runs.
  ///
  /// The developer's own login shell, because that is what every other terminal gives them and
  /// a different one would silently drop their aliases, prompt and path. An argv vector rather
  /// than a command line: §7.3 scopes this as process execution and not a shell, so nothing
  /// here interposes `sh -c` and nothing has to think about quoting.
  const DEFAULT_SHELL = ['/bin/bash', '-l'];

  let starting = $state(false);

  async function startTerminal(): Promise<void> {
    if (starting) return;
    starting = true;
    try {
      // The identity is the client's to choose (FR-001). Time-based rather than counted, so
      // two windows of the same application cannot mint the same one.
      const id = `term-${Date.now()}`;
      const panel = terminals.show(id);
      const fitted = panel.hasTerminal ? panel.fit() : { cols: 80, rows: 24 };
      const started = await taskSink().run(id, DEFAULT_SHELL, fitted.cols, fitted.rows);
      if (!started) {
        // Released rather than left showing an empty terminal that will never fill. A panel
        // for a task that does not exist is the same lie this feature started with.
        terminals.release(id);
      }
    } finally {
      starting = false;
    }
  }

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

      // T079: tell the engine the size **after** attaching. Attaching deliberately has no side
      // effect on the process (attach guarantee 9), so a client that forgets leaves the task
      // laying out to whatever width it had when it started -- which for a reattached build is
      // the width of a window that has since been resized or closed.
      const fitted = panel.fit();
      taskSink().resize(id, fitted.cols, fitted.rows);

      panel.onInput((data) => {
        // A finished task has no stdin to write to. The engine would discard this silently --
        // `writeStdin` is a notification and carries no refusal -- so stopping here is the only
        // place the difference is visible. A native terminal behaves the same way: the buffer
        // stays, scrollback stays, and keys reach nothing.
        if (panel.ending) return;
        // T078. With a terminal the line discipline turns 0x03 into SIGINT for the foreground
        // process group, so an interrupt is just a byte. With pipes there is no line discipline
        // and the same byte is data the task has to parse, so the interrupt must be a signal
        // instead. The panel branches on the shape **it** chose (A-TASKSTREAM).
        if (!panel.hasTerminal && data.includes('\u0003')) {
          taskSink().terminate(id, 'SIGINT');
          return;
        }
        taskSink().writeStdin(id, inputBytes(data));
      });

      panel.onResize((cols, rows) => taskSink().resize(id, cols, rows));
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
    <button class="start" onclick={startTerminal} disabled={starting}>
      {starting ? 'Starting…' : 'Start a terminal'}
    </button>
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
  .transcript:has(.start) {
    display: flex;
    flex-direction: column;
  }
  .prompt-row {
    display: flex;
    gap: var(--vk-term-prompt-gap);
  }
  .prompt {
    color: var(--color-accent);
  }
  .start {
    margin-block-start: var(--vk-term-prompt-gap);
    align-self: flex-start;
    padding: var(--space-1) var(--space-3);
    font: inherit;
    color: var(--color-text);
    background: transparent;
    border: 1px solid var(--color-accent);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .start:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .start:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .cursor {
    inline-size: var(--vk-term-cursor-w);
    block-size: var(--vk-term-cursor-h);
    background: var(--color-accent);
    animation: var(--vk-term-cursor-blink);
  }
</style>
