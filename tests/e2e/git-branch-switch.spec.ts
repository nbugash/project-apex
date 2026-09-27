// T065 — US4. A real branch switch on the host leaves the tree showing the new branch's state
// and nothing from the old (FR-024, FR-025, FR-026, SC-005, SC-011).
//
// The count is in `git_branch_switch.rs`, against the engine where the frames are. What this
// adds is the half that cannot be counted: what a developer is looking at afterwards.
import { spawnSync } from 'node:child_process';
import { WORKSPACE, resetWorkspace, openWorkspace } from './editor-harness';
import { waitForShell, resetSession } from './helpers';

function git(...args: string[]): string {
  const r = spawnSync('git', args, { cwd: WORKSPACE, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')} failed: ${r.stderr}`);
  return r.stdout;
}

async function status(): Promise<{
  branch: { kind: string; name?: string };
  changes: Array<{ path: string; status: string }>;
}> {
  return browser.execute(async () => {
    const fn = (
      window as unknown as {
        __TAURI_INTERNALS__?: { invoke: (c: string, a?: unknown) => Promise<unknown> };
      }
    ).__TAURI_INTERNALS__?.invoke;
    return (await fn?.('git_status')) as never;
  });
}

async function branchLabel(): Promise<string | null> {
  return browser.execute(
    () => document.querySelector('[data-testid="status-branch"]')?.textContent?.trim() ?? null,
  );
}

describe('a branch switch made on the host', () => {
  beforeEach(async () => {
    // `only-on-side` exists and is modified on `side`, and does not exist on `main`. A client
    // that kept the old branch's marks would go on marking a file that is not there.
    resetWorkspace({ 'shared.txt': 'shared\n' });
    git('init', '-q', '-b', 'main');
    git('config', 'user.email', 'test@example.invalid');
    git('config', 'user.name', 'Test');
    git('config', 'commit.gpgsign', 'false');
    git('add', '-A');
    git('commit', '-q', '-m', 'initial');

    git('checkout', '-q', '-b', 'side');
    resetWorkspaceFile('only-on-side.txt', 'side\n');
    git('add', '-A');
    git('commit', '-q', '-m', 'side');
    git('checkout', '-q', 'main');

    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
    await $('[data-testid="tree-row"][data-path="/shared.txt"]').waitForExist({ timeout: 20_000 });
  });

  function resetWorkspaceFile(name: string, body: string): void {
    spawnSync('sh', ['-c', `printf %s '${body}' > '${WORKSPACE}/${name}'`]);
  }

  it('follows the switch and shows the new branch', async () => {
    await browser.waitUntil(async () => (await branchLabel()) === 'main', {
      timeout: 20_000,
      timeoutMsg: 'the status bar never showed main',
    });

    git('checkout', '-q', 'side');

    await browser.waitUntil(async () => (await branchLabel()) === 'side', {
      timeout: 20_000,
      timeoutMsg: 'the branch never followed the switch',
    });
  });

  it('leaves nothing marked from the branch that was left', async () => {
    // SC-011. Marked on `main`, then switched away: the mark describes a file whose content on
    // `side` is whatever `side` says, and keeping it would be confidently wrong.
    resetWorkspaceFile('shared.txt', 'edited on main\n');
    await browser.waitUntil(
      async () => (await status()).changes.some((c) => c.path === '/shared.txt'),
      { timeout: 20_000, timeoutMsg: '/shared.txt was never marked on main' },
    );

    // Discard the edit and switch, so nothing is carried across by the working tree itself.
    git('checkout', '-q', '--', 'shared.txt');
    git('checkout', '-q', 'side');

    await browser.waitUntil(
      async () => {
        const s = await status();
        return s.branch.name === 'side' && !s.changes.some((c) => c.path === '/shared.txt');
      },
      {
        timeout: 20_000,
        timeoutMsg: 'a mark from the previous branch survived the switch',
      },
    );
  });

  it('keeps the workspace usable throughout the switch', async () => {
    // FR-026a: a switch is not a reason to empty the tree. A client that cleared its rows
    // would be worse than one saying its state may have moved on.
    const before = await browser.execute(
      () => document.querySelectorAll('[data-testid="tree-row"]').length,
    );
    expect(before).toBeGreaterThan(0);

    git('checkout', '-q', 'side');
    await browser.waitUntil(async () => (await branchLabel()) === 'side', { timeout: 20_000 });

    const after = await browser.execute(
      () => document.querySelectorAll('[data-testid="tree-row"]').length,
    );
    expect(after).toBeGreaterThan(0);
  });
});
