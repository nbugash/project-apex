// End-to-end harness. Linux only: `tauri-driver` delegates to the platform's WebDriver, and
// macOS provides none for WKWebView (Appendix A, A-E2E).
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import {
  mkdirSync,
  rmSync,
  appendFileSync,
  existsSync,
  readFileSync,
  readdirSync,
} from 'node:fs';
import { join, dirname, basename } from 'node:path';
import { assertCaptureIsNotBlank } from './helpers';

/** Fixed, repo-local profile. Not a temp dir: WDIO workers are separate processes, so an
 *  environment variable set in onPrepare never reaches the specs. A known path both sides
 *  can compute is simpler than plumbing the value through. */
export const E2E_PROFILE = join(process.cwd(), '.e2e-profile');

// The workspace root owns the target directory: F002 made this three crates, and Cargo puts
// every member's artifacts under the root. The old member path still exists on machines that
// built before the split, holding a stale binary — which is why this was green locally and
// red in CI, where a clean checkout has no leftover to fall back on.
const BINARY = join(process.cwd(), 'target/debug/apex-shell');
/** Kill a child and everything it spawned. Requires the child to be detached. */
function killGroup(child: ChildProcess | null): void {
  if (!child?.pid) return;
  try {
    process.kill(-child.pid, 'SIGTERM');
  } catch {
    try {
      child.kill('SIGTERM');
    } catch {
      // already gone
    }
  }
}

/** Wait until nothing answers on the preview URL, so the next step starts clean.
 *
 *  An HTTP check, not a socket bind on 127.0.0.1. Binding an IPv4 address to test
 *  availability reports "free" while a server listens on ::1, which is the address-family
 *  mismatch that had the fidelity gate waiting out its timeout against a live server. */
async function waitForNothingServing(url: string, timeoutMs = 10_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      await fetch(url, { signal: AbortSignal.timeout(1000) });
    } catch {
      return; // nothing answered
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  console.warn(`something was still serving ${url} after teardown`);
}

/** Where a worker records a blank capture for the launcher to find.
 *
 *  A file, not a module-level array: specs run in worker processes and onPrepare and
 *  onComplete run in the launcher, so nothing in memory survives the trip. This is the same
 *  boundary that made an environment variable set in onPrepare invisible to the specs. */
const SHOTS = 'reports/screenshots';

/** Which feature's screenshots this run produces.
 *
 *  The feature map identity (`F002`), not the spec directory number — the two diverge, since
 *  F001's directory is `003-ssh-transport-core`, and naming F001's screenshots "003" would be
 *  actively misleading. The map guarantees identities are never renumbered or reused; the
 *  directory sequence guarantees nothing of the kind.
 *
 *  Derived from the branch so a run on a feature branch files its own screenshots with nothing
 *  to tag and no spec file to move. `APEX_FEATURE` overrides for a run from anywhere else. */
function featureSegment(): string {
  const override = process.env.APEX_FEATURE?.trim();
  if (override) return override;
  try {
    const branch = spawnSync('git', ['rev-parse', '--abbrev-ref', 'HEAD'], {
      encoding: 'utf8',
    }).stdout?.trim();
    const found = branch?.match(/\bF\d{3}\b/)?.[0];
    if (found) return found;
  } catch {
    // fall through to the explicit default below
  }
  // Not on a feature branch. Named rather than blank, so the screenshots are still filed
  // somewhere findable and the directory says why they are not under a feature.
  return 'unassigned';
}

const FEATURE = featureSegment();
const BLANK_LOG = join(SHOTS, '.blank-captures.log');
/** One line per test that started, written by `beforeTest`.
 *
 *  A separate witness from the captures themselves, and deliberately on a different hook.
 *  The count WebdriverIO hands `onComplete` is spec **files** — 21 here, against 65 tests —
 *  so comparing captures against it would accept a run that photographed a third of them.
 *  `beforeTest` knows the real number, and it keeps knowing it when the capture path is
 *  broken, which is the only circumstance this comparison exists for. */
const STARTED_LOG = join(SHOTS, '.tests-started.log');

/** Every PNG under `reports/screenshots`, at any depth.
 *
 *  Recursive rather than a fixed depth. The previous version read exactly one level, and
 *  `${OS}/${FEATURE}/` added a second — which would have counted zero files and failed every
 *  run, or, if someone had loosened the comparison to make the failure stop, passed while
 *  counting nothing. A depth-independent walk cannot be broken by the next path change. */
function capturedFiles(dir: string = SHOTS): string[] {
  if (!existsSync(dir)) return [];
  const found: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) found.push(...capturedFiles(path));
    else if (entry.name.endsWith('.png')) found.push(path);
  }
  return found;
}

let driver: ChildProcess | null = null;
let preview: ChildProcess | null = null;


export const config: WebdriverIO.Config = {
  runner: 'local',
  // `terminal-live.spec.ts` is excluded here and run by `npm run e2e:live`, which sets
  // `APEX_E2E_LIVE`. See onPrepare: an engine in scope changes what the status bar observes.
  specs: process.env.APEX_E2E_LIVE ? ['./terminal-live.spec.ts'] : ['./*.spec.ts'],
  exclude: process.env.APEX_E2E_LIVE ? [] : ['./terminal-live.spec.ts'],
  // The app is a singleton desktop process and there is one driver on one port.
  maxInstances: 1,
  // Point at the driver started in onPrepare. Without an explicit hostname and port, WDIO
  // tries to manage its own browser driver and rejects "wry", which it cannot launch.
  hostname: '127.0.0.1',
  port: 4444,
  path: '/',
  capabilities: [
    // No browserName: tauri-driver matches on `tauri:options` alone and rejects the session
    // when an unrecognised browserName is present in alwaysMatch.
    {
      // @ts-expect-error tauri:options is a tauri-driver extension to the W3C capabilities
      'tauri:options': { application: BINARY },
    },
  ],
  reporters: ['spec'],
  framework: 'mocha',
  mochaOpts: { ui: 'bdd', timeout: 60_000 },
  autoCompileOpts: {
    autoCompile: true,
    tsNodeOpts: { transpileOnly: true, project: './tsconfig.json' },
  },

  onPrepare: async () => {
    // The whole directory, not just the log: onComplete counts the captures this run
    // produced, and files left by a previous one would make that count meaningless — a run
    // that photographed nothing would inherit yesterday's evidence and pass.
    rmSync(SHOTS, { recursive: true, force: true });
    mkdirSync(SHOTS, { recursive: true });

    spawnSync('npm', ['run', 'build'], { stdio: 'inherit' });
    spawnSync('cargo', ['build', '--manifest-path', 'client/core/Cargo.toml'], {
      stdio: 'inherit',
    });
    // The engine, built only when this run is the one that needs it.
    //
    // **Not set for the ordinary suite, and that is the point.** With an engine present the
    // composition root binds the real transport as the connection status source, and
    // `connection-status.spec.ts` drives the *stub* -- so an engine in scope makes that spec
    // watch a source nothing is driving, and it fails on FR-011's five-second budget. The two
    // suites want different applications, so they get different runs rather than a flag one of
    // them has to remember.
    if (process.env.APEX_E2E_LIVE) {
      spawnSync('cargo', ['build', '-p', 'apex-engine', '--bins'], { stdio: 'inherit' });
      // Absolute, because the application's working directory is tauri-driver's, not this one.
      process.env.APEX_LOCAL_ENGINE = join(process.cwd(), 'target/debug/ide-engine');
    }

    // Launch time here is dominated by something that is not the application. On a machine
    // with no desktop session, GTK asks the session bus to activate
    // org.freedesktop.portal.Desktop during window creation, and waits out a ~22 second
    // activation timeout for a service that will never arrive. Measured: 22s to the first
    // log line with the bus reachable, 1s with it pointed at nothing.
    //
    // Pointing the variable at a non-bus is the honest description of this environment
    // rather than a trick: there is no session bus, and saying so up front costs a failed
    // connect instead of a timeout. It does not mask a delay real users see — on a desktop
    // the portal answers immediately — and the application requests no portal services.
    //
    // This is what the runtime task asked for. Sharing one launch across specs was the
    // other option and was not taken: it would trade per-spec isolation, which this suite
    // depends on, for a saving this already delivers.
    process.env.DBUS_SESSION_BUS_ADDRESS = '/dev/null';

    // Must exist and be exported before the driver starts: the app inherits its
    // environment from the driver process.
    rmSync(E2E_PROFILE, { recursive: true, force: true });
    mkdirSync(E2E_PROFILE, { recursive: true });
    process.env.APEX_DATA_DIR = E2E_PROFILE;

    // A debug build loads `devUrl`, so without a server on that port the webview is blank
    // and every selector times out with no clue why. Serving the real built bundle there
    // keeps the dev workflow untouched and avoids a release build per test run.
    // stdio must not be an unread pipe. A full pipe blocks the writer, and a blocked
    // driver makes every WebDriver command hang until the test times out — which reads as
    // 33 unrelated failures rather than one stalled process.
    // detached so the whole process group can be killed. `npx` spawns vite as a child,
    // and killing the npx wrapper alone orphans vite still listening on 1420 — which then
    // blocks the fidelity gate's own server in the next CI step, where it surfaced as
    // "Port 1420 is already in use" followed by thirty seconds of refused connections.
    preview = spawn('npx', ['vite', 'preview', '--port', '1420', '--strictPort'], {
      stdio: 'ignore',
      detached: true,
    });

    // One driver for the whole run. Spawning per session races on the port: every spec
    // after the first fails to bind and the run stalls.
    driver = spawn('tauri-driver', [], { stdio: 'ignore', detached: true });
    await new Promise((resolve) => setTimeout(resolve, 3000));
  },

  beforeTest: async (test) => {
    // Recorded before anything can go wrong with the capture, so a missing screenshot is
    // always visible as a shortfall rather than as a smaller denominator.
    mkdirSync(dirname(STARTED_LOG), { recursive: true });
    appendFileSync(STARTED_LOG, `${test.title}\n`);
  },

  onComplete: async () => {
    // Negative pid kills the process group, not just the wrapper.
    killGroup(driver);
    killGroup(preview);
    driver = null;
    preview = null;
    // And confirm the port actually came back, so the next step does not inherit it.
    await waitForNothingServing('http://localhost:1420');
    // KEEP_E2E_PROFILE leaves the profile and its log in place for diagnosis.
    if (!process.env.KEEP_E2E_PROFILE) {
      rmSync(E2E_PROFILE, { recursive: true, force: true });
    }

    // After teardown, so a blank-window failure never leaks a driver or a profile.
    //
    // `process.exit` rather than a thrown error or `process.exitCode`, because neither
    // gates: WebdriverIO logs an error thrown from a hook and carries on, and the launcher
    // overwrites the exit code with one computed from the test results. Both were tried,
    // and both produced a red message above a green run — the precise failure this check
    // exists to prevent, reproduced in the check itself.
    if (existsSync(BLANK_LOG)) {
      const blanks = readFileSync(BLANK_LOG, 'utf8').trim().split('\n').filter(Boolean);
      rmSync(BLANK_LOG, { force: true });
      if (blanks.length > 0) {
        console.error(
          `\ne2e — ${blanks.length} screenshot(s) showed no rendered window:\n` +
            blanks.map((b) => `  ${b}`).join('\n') +
            '\n\nThe suite passes against a hidden window; a user cannot use one.\n',
        );
        process.exit(1);
      }
    }

    // Then the positive assertion, which is the one the check above cannot make.
    //
    // The blank-capture log records only failures, so its absence means either "nothing
    // was blank" or "nothing was captured" — and those are the same silence. That is the
    // shape of the defect this whole gate exists for: the job ran its entire history
    // photographing nothing, and every assertion about the captures was dead code that
    // reported success. Recording a missing capture tool fixed one route to that silence;
    // an `afterTest` that never runs at all is another, and no amount of failure logging
    // can catch it, because the thing that would do the logging is what is missing.
    //
    // So: one capture per test that ran. Counting rather than existence, because a
    // shortfall means some tests were not photographed, and "some" is as untrustworthy as
    // "none" when the captures are the evidence.
    const captured = capturedFiles().length;
    const started = existsSync(STARTED_LOG)
      ? readFileSync(STARTED_LOG, 'utf8').trim().split('\n').filter(Boolean).length
      : 0;
    rmSync(STARTED_LOG, { force: true });

    if (started === 0) {
      console.error(
        '\ne2e — no test reported starting, so this run proves nothing. Either the suite ' +
          'selected no tests or the hooks are not running.\n',
      );
      process.exit(1);
    }
    if (captured < started) {
      console.error(
        `\ne2e — ${started} test(s) ran but only ${captured} screenshot(s) exist in ` +
          `${SHOTS}.\n\nThe captures are the evidence that a window rendered. A run that ` +
          `produces fewer than it ran cannot support that claim, whatever the assertions ` +
          `above reported.\n`,
      );
      process.exit(1);
    }
    console.log(
      `e2e — ${captured} screenshot(s) for ${started} test(s), filed under ${SHOTS}/<os>/${FEATURE}/.`,
    );
  },

  /**
   * Capture the GUI after every test.
   *
   * Not Playwright: a Tauri window is an embedded platform webview, which Playwright cannot
   * attach to.
   *
   * Captured from X rather than through WebDriver. `saveScreenshot` against
   * WebKitWebDriver times out here (UND_ERR_CLOSED), and with a timeout per test it
   * dominated the run. Capturing the display is also more faithful: it includes the native
   * window, not only the webview viewport.
   *
   * reports/screenshots/ is gitignored by repo convention — these regenerate every run, so
   * diffing them is useless.
   */
  afterTest: async (test) => {
    const os = process.platform; // 'linux' | 'darwin'
    const slug = (s: string): string =>
      s
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-|-$/g, '')
        .slice(0, 60);

    // reports/screenshots/<os>/<feature>/<title>-<description>-<timestamp>.png
    //
    // The title comes from the describe block and the description from the test name, so a
    // directory listing reads as "what was being exercised, then what about it". Falling back
    // to the spec's filename matters: a test declared at the top level of a file has no parent,
    // and without the fallback every such capture would be named `-<description>-…` and sort
    // together under the empty title.
    const title = slug(test.parent || basename(test.file ?? '', '.spec.ts') || 'e2e');
    const stub = slug(test.title);
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    const dir = join(SHOTS, os, FEATURE);
    mkdirSync(dir, { recursive: true });
    const file = join(dir, `${title}-${stub}-${timestamp}.png`);

    const capture = () =>
      os === 'darwin'
        ? spawnSync('screencapture', ['-x', '-o', file], { timeout: 10_000 })
        : spawnSync('import', ['-window', 'root', file], { timeout: 10_000 });

    // A capture tool that is absent or broken is recorded, not warned about. Warning and
    // returning is what let this job run its entire history with no screenshots at all:
    // ImageMagick was never installed, every capture failed, and the suite passed having
    // photographed nothing. The blank-window assertion below was dead code throughout.
    let shot = capture();
    if (shot.error) {
      mkdirSync(dirname(BLANK_LOG), { recursive: true });
      appendFileSync(BLANK_LOG, `${test.title}: could not capture — ${shot.error.message}\n`);
      return;
    }

    // Retry while the capture is blank. A window that has just been shown is mapped before
    // it is painted, so a capture taken immediately after a relaunch can be empty even
    // though nothing is wrong. Waiting for the paint is the difference between an assertion
    // that means something and one that cries wolf on every restart — seventeen of
    // sixty-five captures on the first run after the launch time dropped.
    const PAINT_DEADLINE_MS = 3000;
    const deadline = Date.now() + PAINT_DEADLINE_MS;
    let problem = assertCaptureIsNotBlank(file);
    while (problem && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 150));
      shot = capture();
      if (shot.error) break;
      problem = assertCaptureIsNotBlank(file);
    }

    // Collected rather than thrown: WebdriverIO logs an error thrown from this hook and
    // carries on, so throwing here would produce a red line in the output and a green run —
    // a check that reports without gating. onComplete fails the process.
    if (problem) {
      mkdirSync(dirname(BLANK_LOG), { recursive: true });
      appendFileSync(BLANK_LOG, `${test.title}: ${problem}\n`);
    }
  },

};
