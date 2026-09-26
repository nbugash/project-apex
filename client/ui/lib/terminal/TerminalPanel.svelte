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
  {/if}
</div>

<style>
  .transcript {
    flex: 1;
    overflow: auto;
    padding: var(--vk-term-pad);
    /* The design system's monospace family first, so everything the prototype could show looks
       exactly as it does there -- then a tail of families for the characters it never had to
       consider.

       A terminal is not a label. It renders whatever a program emits, and a developer's shell
       prompt is routinely built from Powerline separators and Nerd Font icons in the private use
       area (U+E000-U+F8FF), which no text font carries. `--vk-mono` cannot be extended because it
       ends in the `monospace` generic, and a generic matches every character: nothing after it is
       ever reached. Hence `--vk-mono-primary`.

       Every name below is a fallback for a codepoint that would otherwise be a blank box, so this
       cannot change how anything the prototype specifies is drawn. The families are the ones
       developers actually install; none is shipped, and where none is present the result is what
       it is today. Shipping a patched font is a design-system decision with a real size cost, and
       a user-settable terminal font is what VS Code and IntelliJ both provide -- neither belongs
       to this feature. */
    font-family:
      var(--vk-mono-primary),
      'Symbols Nerd Font Mono',
      'Symbols Nerd Font',
      'JetBrainsMono Nerd Font',
      'MesloLGS NF',
      'Hack Nerd Font',
      'FiraCode Nerd Font',
      'DejaVu Sans Mono',
      'Noto Color Emoji',
      ui-monospace,
      Menlo,
      monospace;
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
