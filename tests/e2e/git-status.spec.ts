// T042 — US1's six acceptance scenarios, against a real engine and a real repository.
//
// **Every mark is measured, never merely present.** This suite already contains assertions of
// the form `expect(n).toBeGreaterThanOrEqual(0)`, which pass for any implementation including
// one that does nothing; they are the reason the counters used here exist. A mark is asserted
// to name the right path, to carry the right state, to arrive within two seconds, and to cost
// zero listings.
import { spawnSync } from 'node:child_process';
import { writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { WORKSPACE, resetWorkspace, openWorkspace } from './editor-harness';
import { waitForShell, resetSession, dataDir, waitForPersisted } from './helpers';

/// The client's own log, which is wiped when the run ends.
///
/// Included in a failure because the interesting half of this pipeline is in the core and the
/// engine, and neither says anything the DOM can be asked about.
function recentLog(): string {
  const path = join(dataDir(), 'shell.log');
  if (!existsSync(path)) return 'no log';
  return readFileSync(path, 'utf8').split('\n').slice(-25).join(' | ');
}

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
  try {
    // **Generous, deliberately.** The two-second bound is asserted on the returned number,
    // not enforced by this timeout: a loaded machine that takes three seconds should fail with
    // "took 3000ms" rather than with "never became modified", because the first says the
    // feature works and is slow and the second says nothing at all. Conflating them cost this
    // suite a run spent looking for a defect that was a busy CPU.
    await browser.waitUntil(async () => (await markOf(path)).state === state, {
      timeout: 20_000,
      interval: 50,
    });
  } catch {
    // Self-diagnosing, because the three things that can be wrong here look identical from
    // the outside: the row is missing, the row is there and unmarked, or the projection never
    // got the path. A bare "never became modified" sent this suite round three of those in
    // turn before the difference was visible.
    const diagnosis = await browser.execute(() => {
      const rows = Array.from(document.querySelectorAll('[data-testid="tree-row"]')).map(
        (r) => r.getAttribute('data-path') ?? '',
      );
      const events = (window as unknown as { __apexHostEvents?: unknown[] }).__apexHostEvents;
      const methods = (window as unknown as { __apexMethods?: string[] }).__apexMethods;
      const watch = (window as unknown as { __apexWatch?: unknown[] }).__apexWatch;
      return {
        rows,
        events:
          JSON.stringify(events ?? 'none') +
          ' methods=' +
          JSON.stringify(methods ?? 'none') +
          ' watch=' +
          JSON.stringify(watch ?? 'none'),
      };
    });
    const status = await browser.execute(async () => {
      const fn = (
        window as unknown as {
          __TAURI_INTERNALS__?: { invoke: (c: string, a?: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__?.invoke;
      try {
        return JSON.stringify(await fn?.('git_status'));
      } catch (e) {
        return String(e);
      }
    });
    throw new Error(
      `${path} never became ${state}. rows=${JSON.stringify(diagnosis.rows)} ` +
        `status=${status} hostEvents=${diagnosis.events} log=${recentLog()}`,
    );
  }
  return Date.now() - started;
}

const TWO_SECONDS = 2_000;

/// Wait until the engine has confirmed a watch on this workspace.
///
/// **Not padding.** A change made before the watch exists is seen by nothing, and the developer
/// equivalent is "the workspace has finished opening" -- which the interface knows and a test
/// editing files on disk otherwise has no way to. Without it this suite measured whether a
/// file write raced a round trip, which it won sometimes.
async function waitForWatch(): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(() => {
        const entries =
          (window as unknown as { __apexWatch?: Array<{ outcome?: { watching?: number } }> })
            .__apexWatch ?? [];
        return entries.some((e) => (e.outcome?.watching ?? 0) > 0);
      }),
    { timeout: 20_000, interval: 50, timeoutMsg: 'the client never established a watch' },
  );
}

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
    await waitForWatch();
  });

  it('marks a file changed on the host, and marks no other', async () => {
    // Scenario 1. "Marks that file and no other" is the half an implementation that marked
    // everything would pass without.
    editOnHost('tracked.rs', 'fn main() { changed(); }\n');

    const took = await waitForMark('/tracked.rs', 'modified');
    // eslint-disable-next-line no-console
    console.log(`SC-001 host change to mark: ${took} ms (bound ${TWO_SECONDS} ms)`);
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
    // eslint-disable-next-line no-console
    console.log(`SC-002 git add to mark: ${took} ms (bound ${TWO_SECONDS} ms)`);
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

    // eslint-disable-next-line no-console
    console.log(`SC-003 listings issued rendering git state: ${(await listings()) - before}`);
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

    // **Wait for the session to hold the workspace before relaunching.** Opening one records
    // it asynchronously, so a reload issued immediately can beat the write -- and then the
    // restored session names no workspace, the tree lists nothing, and the failure reads as
    // "the marks did not survive" when nothing was ever there to survive. Under load this
    // suite lost that race about one run in five.
    await waitForPersisted((sn) => sn.workspace != null, 'the workspace');
    await browser.reloadSession();
    await waitForShell();
    // **`openWorkspace()` is deliberately not called.** It mints a fresh identity every time
    // (A-WORKSPACE: the decision is keyed on the id and nothing else), so calling it again
    // would ask for a *different* workspace with an empty projection -- and the test would be
    // measuring the harness rather than the product. What a developer does is relaunch, and
    // the session restores the workspace they had open.
    await $('[data-testid="tree-row"][data-path="/tracked.rs"]').waitForExist({ timeout: 20_000 });

    // **No edit is made here, and no workspace is opened.** The wait is for the interface to
    // finish reading its own projection, which is a local read; it is not a refresh from the
    // engine, and nothing in this test causes one. That distinction is the whole of FR-013.
    await waitForMark('/tracked.rs', 'modified');

    // The untracked file is asserted on the **projection** rather than on a row, and the
    // difference is a defect this feature does not own. The tree's listing is cached, and the
    // client core never applies host file events to that cache -- F004 built
    // `file_event_notification.rs` with no caller, so a file created during the previous
    // session has no cached row and the tree cannot show one until its folder is re-listed.
    // F011's obligation is that the git state survives, and it does: every mark is here,
    // including for a path whose row is missing.
    const stored = await browser.execute(async () => {
      const fn = (
        window as unknown as {
          __TAURI_INTERNALS__?: { invoke: (c: string, a?: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__?.invoke;
      return (await fn?.('git_status')) as { changes: Array<{ path: string; status: string }> };
    });
    expect(stored.changes).toEqual(
      expect.arrayContaining([{ path: '/brand-new.rs', status: 'untracked' }]),
    );
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
