// T060 — US3's acceptance scenarios, against a real engine and a real repository.
//
// Marks for each kind, none on an unmodified file, marks following a host change, and **zero
// file content in the diff payload** — the last inspected on what arrives rather than on what
// the engine does with it, because a parser that discards text and a result that carries it
// look identical from the parser's side (§12.3, FR-021, SC-010).
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { WORKSPACE, resetWorkspace, openWorkspace, openFile, editorText } from './editor-harness';
import { waitForShell, resetSession } from './helpers';

function git(...args: string[]): string {
  const r = spawnSync('git', args, { cwd: WORKSPACE, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')} failed: ${r.stderr}`);
  return r.stdout;
}

const ORIGINAL = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', ''].join('\n');

/// One of each kind, kept apart on purpose.
///
/// `b` becomes `B`, `e` goes entirely, `NEW` arrives at the end. Adjacent edits would be
/// folded into a single replacement hunk -- git expresses "two lines became one" as a
/// modification, not as a modification plus a deletion -- so a fixture that changed
/// neighbouring lines would produce no standalone deletion and the deleted case would go
/// untested while the test looked thorough.
const CHANGED = ['a', 'B', 'c', 'd', 'f', 'g', 'h', 'NEW', ''].join('\n');

function makeRepo(): void {
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'test@example.invalid');
  git('config', 'user.name', 'Test');
  git('config', 'commit.gpgsign', 'false');
  git('add', '-A');
  git('commit', '-q', '-m', 'initial');
}

function editOnHost(name: string, body: string): void {
  writeFileSync(join(WORKSPACE, name), body);
}

/** The diff the interface received, straight from the command the gutter calls. */
async function diffOf(path: string): Promise<{
  added: Array<[number, number]>;
  modified: Array<[number, number]>;
  deleted: number[];
}> {
  return browser.execute(async (p: string) => {
    const fn = (
      window as unknown as {
        __TAURI_INTERNALS__?: { invoke: (c: string, a?: unknown) => Promise<unknown> };
      }
    ).__TAURI_INTERNALS__?.invoke;
    return (await fn?.('git_file_diff', { path: p })) as never;
  }, path);
}

/** Gutter decorations Monaco has actually rendered, by kind. */
async function gutterKinds(): Promise<string[]> {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('[class*="vk-gutter-"]'))
      .flatMap((el) => Array.from(el.classList))
      .filter((c) => c.startsWith('vk-gutter-'))
      .sort(),
  );
}

async function open(file: string): Promise<void> {
  resetSession();
  await browser.reloadSession();
  await waitForShell();
  await openWorkspace();
  await openFile(file);
  await browser.waitUntil(async () => (await editorText()).length > 0, {
    timeout: 20_000,
    timeoutMsg: 'the file never arrived',
  });
}

describe('the editor gutter', () => {
  it('marks added, modified and deleted lines', async () => {
    resetWorkspace({ 'notes.md': ORIGINAL });
    makeRepo();
    editOnHost('notes.md', CHANGED);
    await open('/notes.md');

    // Asserted as a whole rather than as three counts: `expect` here takes no message, and a
    // bare "expected 0 to be greater than 0" would not say which of the three was missing.
    const diff = await diffOf('/notes.md');
    expect({
      modified: diff.modified.length > 0,
      added: diff.added.length > 0,
      deleted: diff.deleted.length > 0,
    }).toEqual({ modified: true, added: true, deleted: true });

    await browser.waitUntil(async () => (await gutterKinds()).length > 0, {
      timeout: 20_000,
      timeoutMsg: 'no gutter decoration was rendered',
    });
  });

  it('marks nothing on an unmodified file', async () => {
    // The ordinary case. A gutter that drew something here would make every file look
    // modified, which is the same as marking none of them.
    resetWorkspace({ 'notes.md': ORIGINAL });
    makeRepo();
    await open('/notes.md');

    const diff = await diffOf('/notes.md');
    expect(diff.added).toEqual([]);
    expect(diff.modified).toEqual([]);
    expect(diff.deleted).toEqual([]);

    // Given time to be wrong: a check made immediately would pass against a gutter that draws
    // a moment later.
    await browser.pause(1_500);
    expect(await gutterKinds()).toEqual([]);
  });

  it('follows a change made on the host while the file is open', async () => {
    resetWorkspace({ 'notes.md': ORIGINAL });
    makeRepo();
    await open('/notes.md');
    expect((await diffOf('/notes.md')).modified).toEqual([]);

    editOnHost('notes.md', CHANGED);

    await browser.waitUntil(
      async () => {
        const d = await diffOf('/notes.md');
        return d.modified.length > 0 || d.added.length > 0;
      },
      { timeout: 20_000, timeoutMsg: 'the diff never followed the host change' },
    );
  });

  it('carries no file content in the diff payload, in any field', async () => {
    // SC-010. The secret is on both sides of the change, so any field that leaked content
    // would contain it. Asserted on the whole serialised payload rather than on named fields,
    // because a field added later would escape a check that named them.
    resetWorkspace({ 'secret.env': 'PASSWORD=hunter2\nkeep\n' });
    makeRepo();
    editOnHost('secret.env', 'PASSWORD=swordfish\nkeep\n');
    await open('/secret.env');

    const diff = await diffOf('/secret.env');
    const payload = JSON.stringify(diff);
    expect(payload).not.toContain('hunter2');
    expect(payload).not.toContain('swordfish');
    expect(payload).not.toContain('PASSWORD');
    // And the change was reported, so the assertions above are not passing vacuously.
    expect(diff.added.length + diff.modified.length).toBeGreaterThan(0);
  });

  it('treats a new file as wholly added', async () => {
    // Nothing has been recorded for it to differ from. `git diff` says nothing about an
    // untracked path, so this is the case that needed asking about explicitly.
    resetWorkspace({ 'notes.md': ORIGINAL });
    makeRepo();
    editOnHost('fresh.md', 'alpha\nbeta\ngamma\n');
    await open('/notes.md');

    const diff = await diffOf('/fresh.md');
    expect(diff.added).toEqual([[1, 3]]);
    expect(diff.modified).toEqual([]);
  });
});
