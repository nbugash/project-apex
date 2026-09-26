// Shared ground for the editor's end-to-end specs.
//
// Every spec here runs against a **real engine** reading and writing a **real directory**, and
// the spec process shares that filesystem. That is deliberate and is what makes a conflict
// testable: a colleague's edit is a `writeFileSync` from outside the application, which is
// exactly what a colleague's edit is.

import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

/// A directory of our own, never the repository. A test that writes into the checkout can
/// destroy work and leaves whatever it forgot to clean up behind.
export const WORKSPACE = join(process.cwd(), '.e2e-workspace');

export function resetWorkspace(files: Record<string, string | Buffer>): void {
  rmSync(WORKSPACE, { recursive: true, force: true });
  mkdirSync(join(WORKSPACE, 'src'), { recursive: true });
  for (const [name, body] of Object.entries(files)) {
    writeFileSync(join(WORKSPACE, name), body);
  }
}

/// What is on disk now, which is the only thing that settles whether a write landed.
export function onDisk(name: string): string {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  return require('node:fs').readFileSync(join(WORKSPACE, name), 'utf8');
}

/// Edit a file from outside the application, as a colleague would.
export function editOnHost(name: string, body: string): void {
  writeFileSync(join(WORKSPACE, name), body);
}

async function invoke(command: string, args: unknown): Promise<unknown> {
  return browser.execute(
    async (c: string, a: unknown) => {
      const fn = (
        window as unknown as {
          __TAURI_INTERNALS__?: { invoke: (c: string, a: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__?.invoke;
      return fn?.(c, a);
    },
    command,
    args,
  );
}

/// Register the scratch directory as the workspace. Awaited, because the engine refuses every
/// file request for a workspace it has not been told about.
export async function openWorkspace(): Promise<void> {
  await invoke('workspace_open', { name: 'apex', host: 'localhost', basePath: WORKSPACE });
}

/// Open a file the way a developer does: by clicking it in the tree.
///
/// Through the tree rather than through `documents_open`, because "clicking a file opens it"
/// was itself missing until this feature — the row called `toggle`, which does nothing for a
/// file. A spec that invoked the command directly would have passed against that.
export async function openFile(path: string): Promise<void> {
  // Expand each ancestor first. The tree is lazy by design — a folder's children are fetched
  // the first time it is expanded (FR-014) — so a nested file has no row until its folders do.
  const parts = path.split('/').filter(Boolean);
  for (let i = 1; i < parts.length; i += 1) {
    const folder = `/${parts.slice(0, i).join('/')}`;
    const dir = await $(`[data-testid="tree-row"][data-path="${folder}"]`);
    await dir.waitForDisplayed({ timeout: 20_000 });
    if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.click();
  }

  const row = await $(`[data-testid="tree-row"][data-path="${path}"]`);
  await row.waitForDisplayed({ timeout: 20_000 });
  await row.click();
  await browser.waitUntil(async () => (await $('[data-testid="editor"]')).isExisting(), {
    timeout: 20_000,
    timeoutMsg: `the editor never opened for ${path}`,
  });
}

/// What the editor is showing, from Monaco itself rather than from our model.
export async function editorText(): Promise<string> {
  return (await browser.execute(() => {
    const e = (window as unknown as { __apexEditor?: { getValue: () => string } }).__apexEditor;
    return e?.getValue() ?? '';
  })) as string;
}

/// Put the caret in the editor and type, as keystrokes rather than as a value assignment.
export async function typeInEditor(text: string): Promise<void> {
  await browser.execute(() => {
    const e = (window as unknown as { __apexEditor?: { focus: () => void } }).__apexEditor;
    e?.focus();
  });
  // One call per character, rather than `browser.keys(text.split(''))`.
  //
  // The array form is a single WebDriver action sequence, and it loses characters: typing
  // `// offline edit` through it produced `/ oflinedit` reproducibly. Separate calls are slower
  // and are what makes SC-009 -- every character typed while disconnected is still present --
  // an assertion about the application rather than about the driver.
  for (const ch of text) await browser.keys(ch);
}

/// Every request the editor has issued, as the sink recorded them leaving.
export async function requestsIssued(): Promise<Array<{ method: string; path: string }>> {
  return (await browser.execute(
    () =>
      (window as unknown as { __apexEditorSent?: Array<{ method: string; path: string }> })
        .__apexEditorSent ?? [],
  )) as Array<{ method: string; path: string }>;
}

export async function clickSave(): Promise<void> {
  await (await $('[data-testid="editor-save"]')).click();
}

/// Wait for the save bar to report an ending, and say which it was.
export async function endingTone(): Promise<string> {
  const notice = await $('[data-testid="editor-ending"]');
  await notice.waitForDisplayed({ timeout: 20_000 });
  return (await notice.getAttribute('data-tone')) ?? '';
}
