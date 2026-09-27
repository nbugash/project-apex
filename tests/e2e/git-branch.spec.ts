// T049 — US2's acceptance scenarios, against a real engine and a real repository.
//
// The branch shows, follows a switch, identifies the commit when HEAD is detached, and is
// **absent** for a workspace that is not a repository — while that workspace stays fully usable
// and surfaces no errors at all (SC-007). The engine's half of that last claim is
// `git_degrade.rs`; a successful empty status still reaches a client that could render it as a
// failure, and only this says whether it does.
import { spawnSync } from 'node:child_process';
import { WORKSPACE, resetWorkspace, openWorkspace } from './editor-harness';
import { waitForShell, resetSession } from './helpers';

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

async function open(): Promise<void> {
  resetSession();
  await browser.reloadSession();
  await waitForShell();
  await openWorkspace();
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
    resetWorkspace({ 'plain.txt': 'no repository here\n' });
    await open();

    // Given time to be wrong: the assertion is that nothing appears, and a check made
    // immediately would pass against an indicator that arrives a moment later.
    await browser.pause(2_000);
    expect(await indicator()).toBeNull();
  });

  it('leaves a workspace that is not a repository fully usable and error-free', async () => {
    resetWorkspace({ 'plain.txt': 'no repository here\n', 'other.txt': 'also fine\n' });
    await open();
    await browser.pause(2_000);

    expect(await errorsOnScreen()).toBe(0);
    const rows = await browser.execute(
      () =>
        Array.from(document.querySelectorAll('[data-testid="tree-row"]')).map(
          (r) => r.getAttribute('data-path') ?? '',
        ),
    );
    expect(rows).toEqual(expect.arrayContaining(['/plain.txt', '/other.txt']));
  });
});
