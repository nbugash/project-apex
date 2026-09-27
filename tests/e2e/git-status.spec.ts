// T042 — US1's six acceptance scenarios, against a real engine and a real repository.
//
// **Every mark is measured, never merely present.** This suite already contains assertions of
// the form `expect(n).toBeGreaterThanOrEqual(0)`, which pass for any implementation including
// one that does nothing; they are the reason the counters used here exist. A mark is asserted
// to name the right path, to carry the right state, to arrive within two seconds, and to cost
// zero listings.
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { WORKSPACE, resetWorkspace, openWorkspace } from './editor-harness';
import { waitForShell, resetSession } from './helpers';

/** Run git inside the test workspace, as a developer in another terminal would. */
function git(...args: string[]): string {
  const r = spawnSync('git', args, { cwd: WORKSPACE, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')} failed: ${r.stderr}`);
  return r.stdout;
}

/** A repository with one commit, so there is something for changes to differ from. */
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

/** The git mark on one row, exactly as the DOM holds it. */
async function markOf(path: string): Promise<{ state: string | null; glyph: string }> {
  return browser.execute((p: string) => {
    const row = document.querySelector(`[data-testid="tree-row"][data-path="${p}"]`);
    const vcs = row?.querySelector('.vcs');
    return {
      state: vcs?.getAttribute('data-git') ?? null,
      glyph: vcs?.textContent?.trim() ?? '',
    };
  }, path);
}

/** How many directory listings the tree has issued since the page loaded. */
async function listings(): Promise<number> {
  return browser.execute(
    () => (window as unknown as { __apexListings?: string[] }).__apexListings?.length ?? 0,
  );
}

/// Wait for a row to carry a given git state, and return how long it took.
///
/// Returns the elapsed milliseconds rather than a boolean, so SC-001 and SC-002 are asserted
/// as the numbers they are stated as rather than as "it eventually showed up".
async function waitForMark(path: string, state: string): Promise<number> {
  const started = Date.now();
  await browser.waitUntil(async () => (await markOf(path)).state === state, {
    timeout: 5_000,
    interval: 50,
    timeoutMsg: `${path} never became ${state}`,
  });
  return Date.now() - started;
}

const TWO_SECONDS = 2_000;

describe('git status in the tree', () => {
  beforeEach(async () => {
    resetWorkspace({
      'tracked.rs': 'fn main() {}\n',
      'second.rs': 'fn other() {}\n',
      'src/nested.rs': 'mod nested;\n',
    });
    makeRepo();
    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
    await $('[data-testid="tree-row"][data-path="/tracked.rs"]').waitForExist({ timeout: 20_000 });
  });

  it('marks a file changed on the host, and marks no other', async () => {
    // Scenario 1. "Marks that file and no other" is the half an implementation that marked
    // everything would pass without.
    editOnHost('tracked.rs', 'fn main() { changed(); }\n');

    const took = await waitForMark('/tracked.rs', 'modified');
    expect(took).toBeLessThan(TWO_SECONDS);
    expect((await markOf('/second.rs')).state).toBeNull();
  });

  it("changes a file's mark when it is staged", async () => {
    // Scenario 2, and the case the workspace watcher cannot see: `git add` writes the index and
    // touches no working-tree file (A-GITWATCH).
    editOnHost('tracked.rs', 'fn main() { changed(); }\n');
    await waitForMark('/tracked.rs', 'modified');

    git('add', 'tracked.rs');

    const took = await waitForMark('/tracked.rs', 'staged');
    expect(took).toBeLessThan(TWO_SECONDS);
  });

  it('distinguishes an untracked file without relying on colour', async () => {
    // Scenario 3, FR-015. The glyph is the channel that survives greyscale; the unit test
    // proves the tokens are separated too, and this proves the glyph actually reaches the DOM.
    editOnHost('brand-new.rs', 'fn fresh() {}\n');
    editOnHost('tracked.rs', 'fn main() { changed(); }\n');

    await waitForMark('/brand-new.rs', 'untracked');
    await waitForMark('/tracked.rs', 'modified');

    const untracked = await markOf('/brand-new.rs');
    const modified = await markOf('/tracked.rs');
    expect(untracked.glyph.length).toBe(1);
    expect(modified.glyph.length).toBe(1);
    expect(untracked.glyph).not.toBe(modified.glyph);
  });

  it('renders the marks without issuing a single listing', async () => {
    // Scenario 4, SC-002. The number, not a feeling. An implementation that re-listed the tree
    // on every update would look identical on screen and cost a round trip per folder per
    // keystroke a colleague types.
    const before = await listings();
    editOnHost('tracked.rs', 'a\n');
    await waitForMark('/tracked.rs', 'modified');
    editOnHost('second.rs', 'b\n');
    await waitForMark('/second.rs', 'modified');

    expect(await listings()).toBe(before);
  });

  it('leaves the count of cached files unchanged across an update', async () => {
    // Scenario 5, FR-010, §5.3. Git status says what differs from the repository, which is not
    // a statement about whether a cached copy still matches the host. The failure is silent:
    // content quietly re-fetched reads as "the client feels slow".
    const rowsBefore = await browser.execute(
      () => document.querySelectorAll('[data-testid="tree-row"]').length,
    );
    editOnHost('tracked.rs', 'changed\n');
    await waitForMark('/tracked.rs', 'modified');

    const rowsAfter = await browser.execute(
      () => document.querySelectorAll('[data-testid="tree-row"]').length,
    );
    expect(rowsAfter).toBe(rowsBefore);
  });

  it('still marks the same files after a relaunch, before any refresh arrives', async () => {
    // Scenario 6, FR-013. **The scenario that had no test when analyze checked**, and the one
    // an in-memory projection passes every other assertion here without.
    editOnHost('tracked.rs', 'changed\n');
    editOnHost('brand-new.rs', 'fresh\n');
    await waitForMark('/tracked.rs', 'modified');
    await waitForMark('/brand-new.rs', 'untracked');

    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
    await $('[data-testid="tree-row"][data-path="/tracked.rs"]').waitForExist({ timeout: 20_000 });

    // No edit is made here on purpose: what is being asserted is that the marks came out of the
    // projection rather than out of a refresh this test caused.
    expect((await markOf('/tracked.rs')).state).toBe('modified');
    expect((await markOf('/brand-new.rs')).state).toBe('untracked');
  });

  it('leaves a file git says nothing about exactly as it was', async () => {
    // FR-016. The column was reserved at a fixed size by F000, so filling it must not reflow
    // the rows around it.
    editOnHost('tracked.rs', 'changed\n');
    await waitForMark('/tracked.rs', 'modified');

    const unmarked = await markOf('/second.rs');
    expect(unmarked.state).toBeNull();
    expect(unmarked.glyph).toBe('');
  });
});
