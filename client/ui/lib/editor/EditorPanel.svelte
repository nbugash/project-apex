<script lang="ts">
  /**
   * One file's editor.
   *
   * # What is imported, and why it is not all of Monaco
   *
   * `editor.api` is the core: text model, rendering, selection, undo. The language
   * *contributions* imported beside it are Monarch tokenizers — lexical, synchronous, on the main
   * thread — which is what gives syntax colour and what §8.1 wants to stay local so colour
   * survives a dropped connection (FR-005).
   *
   * What is deliberately **not** imported is `language/typescript` and its siblings, which spawn
   * web workers and answer completion, hover and diagnostics from the single file in the browser.
   * Those answers would be about a local model of a remote workspace, and a suggestion that looks
   * real but is not is worse than none (FR-004). F007 supplies them from the engine's language
   * servers.
   *
   * # Why the model is not in this component
   *
   * This component is unmounted whenever its tab is not focused. The buffer, its base hash and
   * its dirty flag live in `buffers.svelte.ts` for that reason.
   */
  import { onMount } from 'svelte';
  import {
    AUTOSAVE_DEBOUNCE_MS,
    buffers,
    load,
    loadRest,
    loadWindow,
    save,
    type Buffer,
  } from './buffers.svelte';
  import { describeOutcome } from './ending';
  import { editorTheme } from './palette';
  import { sessionSetAutosave } from '../ipc';

  interface Props {
    /// The path whose buffer to show, or null for no document.
    path?: string | null;
    /// Whether saves happen without being asked. Owned by the session store, passed in, so this
    /// component never becomes a second place the preference lives (FR-007b).
    autosave?: boolean;
  }
  let { path = null, autosave = false }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  let editor: import('monaco-editor/editor/editor.api').editor.IStandaloneCodeEditor | null =
    null;
  let monaco: typeof import('monaco-editor/editor/editor.api') | null = null;
  let applying = false;
  let autosaveTimer: ReturnType<typeof setTimeout> | null = null;

  const THEME = 'apex';

  /// The buffer on screen, or null.
  ///
  /// A **read**, never a create. `ensure` pushes onto the buffer set, and Svelte 5 refuses a
  /// state mutation inside a derived — which does not fail quietly: the whole shell stopped
  /// mounting the moment a tab was focused, and `.shell` never appeared. Creating belongs in
  /// the effect below, where a mutation is legal and where the read that follows it still sees
  /// the new buffer, because the set is itself state.
  const buffer = $derived<Buffer | null>(path ? (buffers.get(path) ?? null) : null);

  /// What the last save produced, in the developer's terms.
  const ending = $derived(buffer?.ending ? describeOutcome(buffer.ending) : null);

  /// Monaco's own id for a language, from the file's suffix. Only the languages whose
  /// contributions are imported below are worth naming; anything else renders as plain text,
  /// which is correct rather than a gap.
  function languageFor(p: string): string {
    const ext = p.slice(p.lastIndexOf('.') + 1).toLowerCase();
    const known: Record<string, string> = {
      rs: 'rust',
      ts: 'typescript',
      js: 'javascript',
      json: 'json',
      md: 'markdown',
      css: 'css',
      html: 'html',
      py: 'python',
      sh: 'shell',
      toml: 'ini',
      yml: 'yaml',
      yaml: 'yaml',
    };
    return known[ext] ?? 'plaintext';
  }

  onMount(() => {
    let disposed = false;
    void (async () => {
      const [api] = await Promise.all([
        import('monaco-editor/editor/editor.api'),
        // Every Monarch tokenizer in one entry, which is how 0.57 packages them. Lexical,
        // synchronous, main-thread: `grep -c worker` over this module returns zero. That is the
        // whole reason it is safe to import while `language/typescript` and its three siblings
        // are not — those start workers and answer questions about a workspace they cannot see.
        import('monaco-editor/basic-languages/monaco.contribution'),
      ]);
      if (disposed || !host) return;
      monaco = api;
      api.editor.defineTheme(THEME, editorTheme(host) as never);
      editor = api.editor.create(host, {
        value: buffer?.text ?? '',
        language: path ? languageFor(path) : 'plaintext',
        theme: THEME,
        automaticLayout: true,
        minimap: { enabled: false },
        // The font is the design system's, read from the element so the stylesheet stays the one
        // place it is decided — the same rule the terminal follows after its metrics defect.
        fontFamily: getComputedStyle(host).fontFamily,
        fontSize: Number.parseFloat(getComputedStyle(host).fontSize) || undefined,
        scrollBeyondLastLine: false,
        readOnly: buffer ? !buffer.editable : true,
      });

      editor.onDidChangeModelContent(() => {
        if (applying || !buffer || !editor) return;
        const next = editor.getValue();
        if (!buffer.applyEdit(next)) {
          // Refused because the file is not wholly loaded. Put the model back rather than
          // leaving the screen showing an edit the buffer rejected.
          applying = true;
          editor.setValue(buffer.text);
          applying = false;
          return;
        }
        scheduleAutosave();
      });

      // Fetch the next window when the viewport reaches the end of what is held (FR-017).
      // Anchored on the bottom of the scrollable area rather than on a line number: the buffer
      // grows by appending windows, so what is loaded is always a prefix, and "near the end of
      // the text" is exactly "near the end of what has been fetched".
      editor.onDidScrollChange(() => {
        if (!editor || !buffer || buffer.editable) return;
        const layout = editor.getLayoutInfo();
        const remaining = editor.getScrollHeight() - editor.getScrollTop() - layout.height;
        if (remaining > layout.height) return;
        const missing = buffer.loaded.missingFor(0, buffer.loaded.total);
        if (missing.length === 0) return;
        void loadWindow(buffer, buffer.loaded.total, missing[0]![0]);
      });

      publishForAutomation(editor);
    })();
    return () => {
      disposed = true;
      if (autosaveTimer !== null) clearTimeout(autosaveTimer);
      editor?.dispose();
      editor = null;
    };
  });

  /// Make sure a buffer exists for this tab, and fill it the first time.
  ///
  /// A restored tab has a buffer before it has content (FR-021). Fetched when the tab is
  /// focused, which is when this component mounts, rather than for every tab at launch —
  /// restoring ten tabs would otherwise be ten reads of files nobody is looking at.
  $effect(() => {
    if (!path) return;
    const b = buffers.ensure(path);
    if (b.base === null && b.notice === null && !b.loading) void load(b);
  });

  /// Follow the buffer when the **host** changes it: a tab switch, a reload, an appended window.
  ///
  /// Keyed on the revision rather than on whether the two texts differ. Typing makes them differ
  /// constantly and momentarily, so an effect that corrected the model whenever it noticed would
  /// throw away keystrokes and move the caret.
  let appliedRevision = -1;
  $effect(() => {
    const b = buffer;
    if (!editor || !monaco || !b) return;
    if (b.revision !== appliedRevision) {
      appliedRevision = b.revision;
      if (editor.getValue() !== b.text) {
        applying = true;
        editor.setValue(b.text);
        applying = false;
      }
    }
    editor.updateOptions({ readOnly: !b.editable });
    if (path) {
      const model = editor.getModel();
      if (model) monaco.editor.setModelLanguage(model, languageFor(path));
    }
  });

  /// Expose the editor so the end-to-end suite can read what is rendered, on the same terms as
  /// the terminal: only under automation, because what a developer has open is their business.
  function publishForAutomation(
    e: import('monaco-editor/editor/editor.api').editor.IStandaloneCodeEditor,
  ): void {
    if (!(import.meta.env.DEV || navigator.webdriver === true)) return;
    (window as unknown as Record<string, unknown>).__apexEditor = e;
  }

  /// Save what is in the buffer. Explicit by default, because every save is a chance for a
  /// conflict and a deliberate save keeps refusals meaningful (spec.md, *Clarifications*).
  export async function saveNow(): Promise<void> {
    if (buffer) await save(buffer);
  }

  /// Start the clock again on every keystroke (FR-007c).
  ///
  /// Restarted rather than left running, so a write happens once after typing stops instead of
  /// every two seconds while it continues. `save` itself declines a buffer with no changes, so
  /// a timer that fires after a manual save costs nothing.
  function scheduleAutosave(): void {
    if (!autosave) return;
    if (autosaveTimer !== null) clearTimeout(autosaveTimer);
    autosaveTimer = setTimeout(() => {
      autosaveTimer = null;
      if (buffer) void save(buffer);
    }, AUTOSAVE_DEBOUNCE_MS);
  }

  async function toggleAutosave(event: Event): Promise<void> {
    const on = (event.currentTarget as HTMLInputElement).checked;
    await sessionSetAutosave(on);
    if (!on && autosaveTimer !== null) {
      clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
  }

  /// Discard local changes and take the host's content. The one escape from a conflict this
  /// feature offers (FR-012a); there is deliberately no counterpart that overwrites (FR-012b).
  async function discardAndReload(): Promise<void> {
    if (!buffer) return;
    buffer.dirty = false;
    await load(buffer);
  }
</script>

<div class="panel">
  {#if buffer?.notice}
    <p class="notice" role="status" data-testid="editor-notice" data-kind={buffer.notice.kind}>
      {#if buffer.notice.kind === 'binary'}
        This file is not text, so it cannot be shown here. Viewing it is F017's job.
      {:else if buffer.notice.kind === 'tooLarge'}
        This file is {Math.round(buffer.notice.total / (1024 * 1024))} MB, past the {Math.round(
          buffer.notice.limit / (1024 * 1024),
        )} MB this editor opens.
      {:else if buffer.notice.kind === 'missing'}
        This file no longer exists on the host. Your changes are still here.
      {:else if buffer.notice.kind === 'diverged'}
        This file changed on the host while you were editing it. Your changes are still here.
      {:else}
        This file could not be read: {buffer.notice.message}
      {/if}
    </p>
  {/if}

  {#if ending}
    <p class="notice" role="status" data-testid="editor-ending" data-tone={ending.tone}>
      <span>{ending.title}</span>
      {#if ending.detail}<span class="detail">{ending.detail}</span>{/if}
      {#if ending.offersReload}
        <button type="button" onclick={discardAndReload} data-testid="editor-discard">
          Discard my changes and load the host's version
        </button>
      {/if}
    </p>
  {/if}

  {#if buffer && !buffer.editable && buffer.base !== null}
    <p class="notice" role="status" data-testid="editor-partial">
      <span>Only part of this file is loaded, so it cannot be edited yet.</span>
      <button type="button" onclick={() => buffer && loadRest(buffer)} data-testid="editor-load-rest">
        Load the rest
      </button>
    </p>
  {/if}

  <div class="editor" bind:this={host} data-testid="editor" data-path={path ?? ''}></div>

  <div class="bar">
    <label>
      <input
        type="checkbox"
        checked={autosave}
        onchange={toggleAutosave}
        data-testid="editor-autosave"
      />
      Save automatically
    </label>
    <button type="button" onclick={saveNow} data-testid="editor-save" disabled={!buffer?.dirty}>
      Save
    </button>
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-block-size: 0;
    min-inline-size: 0;
  }

  .editor {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    font-family: var(--vk-mono);
    font-size: var(--vk-code);
    line-height: var(--vk-line);
  }

  /* Spacing and colour from the design system's own tokens rather than from numbers measured
     off an adjacent surface. `lint:ds` refused an earlier version of this block that invented
     `--vk-gap-*` names, which is the check working: FR-022 wants a gap resolved with the
     designer, not improvised from whatever was nearby. Nothing here is a new value -- the
     status bar's own size token is reused for text that sits at the same level of the
     hierarchy. */
  .notice {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    flex-wrap: wrap;
    margin: 0;
    padding: var(--space-2);
    background: var(--color-surface);
    color: var(--color-text);
    font-family: var(--font-body);
    font-size: var(--vk-status-size);
  }

  .detail {
    color: var(--color-neutral-300);
  }

  .bar {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    justify-content: flex-end;
    padding: var(--space-1) var(--space-2);
    border-block-start: 1px solid var(--color-divider);
    font-family: var(--font-body);
    font-size: var(--vk-status-size);
  }
</style>
