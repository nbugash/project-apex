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
  import { buffers, type Buffer } from './buffers.svelte';
  import { editorTheme } from './palette';
  import { editorSink } from './sink';

  interface Props {
    /// The path whose buffer to show, or null for no document.
    path?: string | null;
  }
  let { path = null }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  let editor: import('monaco-editor/editor/editor.api').editor.IStandaloneCodeEditor | null =
    null;
  let monaco: typeof import('monaco-editor/editor/editor.api') | null = null;
  let applying = false;

  const THEME = 'apex';

  /// The buffer on screen, or null. Derived so the component follows the model rather than
  /// holding a copy of it.
  const buffer = $derived<Buffer | null>(path ? (buffers.get(path) ?? null) : null);

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
        }
      });
      publishForAutomation(editor);
    })();
    return () => {
      disposed = true;
      editor?.dispose();
      editor = null;
    };
  });

  /// Follow the buffer when the tab changes or the content is reloaded from the host.
  $effect(() => {
    const b = buffer;
    if (!editor || !monaco || !b) return;
    if (editor.getValue() !== b.text) {
      applying = true;
      editor.setValue(b.text);
      applying = false;
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

  /// Save what is in the buffer. Explicit, because every save is a chance for a conflict and a
  /// deliberate save keeps refusals meaningful (spec.md, *Clarifications*).
  export async function save(): Promise<void> {
    const b = buffer;
    if (!b || !b.base || b.saving || !b.dirty) return;
    b.saving = true;
    try {
      const outcome = await editorSink().write(b.path, b.text, b.base);
      if (outcome.kind === 'written') b.adopt(outcome.sha256);
      else b.failed(outcome);
    } finally {
      b.saving = false;
    }
  }
</script>

<div class="editor" bind:this={host} data-testid="editor" data-path={path ?? ''}></div>

<style>
  .editor {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    font-family: var(--vk-mono);
    font-size: var(--vk-code);
    line-height: var(--vk-line);
  }
</style>
