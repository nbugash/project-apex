// T049 — US2's acceptance scenarios, against a real engine and a real repository.
//
// The branch shows, follows a switch, identifies the commit when HEAD is detached, and is
// **absent** for a workspace that is not a repository — while that workspace stays fully usable
// and surfaces no errors at all (SC-007). The engine's half of that last claim is
// `git_degrade.rs`; a successful empty status still reaches a client that could render it as a
// failure, and only this says whether it does.
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WORKSPACE, resetWorkspace, openWorkspace } from './editor-harness';
import { waitForShell, resetSession } from './helpers';

/// A directory that is genuinely inside no repository.
///
/// **Not `.e2e-workspace`**, which lives inside this project's own checkout: under FR-003a a
/// workspace there is served as a subtree of *this* repository and correctly shows its branch.
/// The earlier version of these two tests used it and passed only because subdirectory
/// workspaces were refused outright; when that changed they failed, which is the test doing its
/// job. A non-repository has to be somewhere outside the tree.
function outsideAnyRepository(files: Record<string, string>): string {
  const dir = mkdtempSync(join(tmpdir(), 'apex-nonrepo-'));
  for (const [name, body] of Object.entries(files)) writeFileSync(join(dir, name), body);
  return dir;
}

function git(...args: string[]): string {
  const r = spawnSync('git', args, { cwd: WORKSPACE, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')} failed: ${r.stderr}`);
  return r.stdout;
}

function makeRepo(): void {
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'test@example.invalid');
  git('config', 'user.name', 'Test');
  git('config', 'commit.gpgsign', 'false');
  git('add', '-A');
  git('commit', '-q', '-m', 'initial');
}

/** The branch indicator, or `null` when it is not on the bar at all. */
async function indicator(): Promise<{ text: string; kind: string | null; title: string } | null> {
  return browser.execute(() => {
    const el = document.querySelector('[data-testid="status-branch"]');
    if (!el) return null;
    return {
      text: el.textContent?.trim() ?? '',
      kind: el.getAttribute('data-kind'),
      title: el.getAttribute('title') ?? '',
    };
  });
}

/** Anything the interface is showing as a failure. */
async function errorsOnScreen(): Promise<number> {
  return browser.execute(
    () =>
      document.querySelectorAll(
        '[role="alert"], [role="alertdialog"], [data-testid="tree-problem"]',
      ).length,
  );
}

async function waitForBranch(text: string): Promise<void> {
  await browser.waitUntil(async () => (await indicator())?.text === text, {
    timeout: 20_000,
    interval: 50,
    timeoutMsg: `the status bar never showed ${text}`,
  });
}

async function open(base?: string): Promise<void> {
  resetSession();
  await browser.reloadSession();
  await waitForShell();
  await openWorkspace(base);
  await $('[data-testid="tree-row"]').waitForExist({ timeout: 20_000 });
}

describe('the branch indicator', () => {
  it('names the branch the workspace is on', async () => {
    resetWorkspace({ 'a.rs': 'fn a() {}\n' });
    makeRepo();
    await open();
    await waitForBranch('main');
    expect((await indicator())?.kind).toBe('branch');
  });

  it('follows a branch switch made on the host', async () => {
    // Nothing is asked for: `HEAD` changing is one of A-GITWATCH's two watches, and this is
    // the case it exists for.
    resetWorkspace({ 'a.rs': 'fn a() {}\n' });
    makeRepo();
    await open();
    await waitForBranch('main');

    git('checkout', '-q', '-b', 'elsewhere');

    await waitForBranch('elsewhere');
  });

  it('identifies the commit when HEAD is detached, and calls it nothing else', async () => {
    // The defect a header-as-name reading produces is a branch apparently called `(detached)`,
    // which a developer mid-rebase would believe and try to push.
    resetWorkspace({ 'a.rs': 'fn a() {}\n' });
    makeRepo();
    const head = git('rev-parse', 'HEAD').trim();
    git('checkout', '-q', '--detach', head);
    await open();

    await browser.waitUntil(async () => (await indicator())?.kind === 'detached', {
      timeout: 20_000,
      timeoutMsg: 'the status bar never reported a detached head',
    });
    const shown = await indicator();
    expect(shown?.text).toBe(head.slice(0, 7));
    expect(shown?.text).not.toContain('detached');
    // Shortened for the bar, whole where it can be read.
    expect(shown?.title).toContain(head);
  });

  it('shows nothing at all for a workspace that is not a repository', async () => {
    // FR-019 and SC-007 together. A placeholder would make a statement about git to a
    // developer who is not using it, and an error would say the workspace had failed.
    const dir = outsideAnyRepository({ 'plain.txt': 'no repository here\n' });
    try {
      await open(dir);
      // Given time to be wrong: the assertion is that nothing appears, and a check made
      // immediately would pass against an indicator that arrives a moment later.
      await browser.pause(2_000);
      expect(await indicator()).toBeNull();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('leaves a workspace that is not a repository fully usable and error-free', async () => {
    const dir = outsideAnyRepository({
      'plain.txt': 'no repository here\n',
      'other.txt': 'also fine\n',
    });
    try {
      await open(dir);
      await browser.pause(2_000);

      expect(await errorsOnScreen()).toBe(0);
      const rows = await browser.execute(
        () =>
          Array.from(document.querySelectorAll('[data-testid="tree-row"]')).map(
            (r) => r.getAttribute('data-path') ?? '',
          ),
      );
      expect(rows).toEqual(expect.arrayContaining(['/plain.txt', '/other.txt']));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('serves a workspace on a subdirectory as a subtree of its repository', async () => {
    // FR-003a through the interface. The branch is the repository's; the marks are only this
    // subtree's, re-rooted. Without the re-rooting every mark names a path that does not exist
    // here, and the developer sees an accurate branch beside a tree claiming nothing changed.
    resetWorkspace({ 'top.txt': 'at the root\n' });
    makeRepo();
    spawnSync('sh', ['-c', `mkdir -p '${WORKSPACE}/inner' && printf 'x' > '${WORKSPACE}/inner/deep.txt'`]);
    git('add', '-A');
    git('commit', '-q', '-m', 'add inner');
    spawnSync('sh', ['-c', `printf 'changed' > '${WORKSPACE}/inner/deep.txt'`]);
    spawnSync('sh', ['-c', `printf 'changed' > '${WORKSPACE}/top.txt'`]);

    await open(`${WORKSPACE}/inner`);

    await browser.waitUntil(async () => (await indicator())?.kind === 'branch', {
      timeout: 20_000,
      timeoutMsg: 'a subdirectory workspace showed no branch',
    });

    const status = await browser.execute(async () => {
      const fn = (
        window as unknown as {
          __TAURI_INTERNALS__?: { invoke: (c: string, a?: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__?.invoke;
      return (await fn?.('git_status')) as { changes: Array<{ path: string }> };
    });
    const paths = status.changes.map((c) => c.path);
    expect(paths).toContain('/deep.txt');
    expect(paths.some((p) => p.includes('top.txt'))).toBe(false);
  });
});
