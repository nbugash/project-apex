// End-to-end harness. Linux only: `tauri-driver` delegates to the platform's WebDriver, and
// macOS provides none for WKWebView (Appendix A, A-E2E).
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';

/** Fixed, repo-local profile. Not a temp dir: WDIO workers are separate processes, so an
 *  environment variable set in onPrepare never reaches the specs. A known path both sides
 *  can compute is simpler than plumbing the value through. */
export const E2E_PROFILE = join(process.cwd(), '.e2e-profile');

const BINARY = join(process.cwd(), 'src-tauri/target/debug/apex-shell');
let driver: ChildProcess | null = null;
let preview: ChildProcess | null = null;


export const config: WebdriverIO.Config = {
  runner: 'local',
  specs: ['./*.spec.ts'],
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
    spawnSync('npm', ['run', 'build'], { stdio: 'inherit' });
    spawnSync('cargo', ['build', '--manifest-path', 'src-tauri/Cargo.toml'], {
      stdio: 'inherit',
    });

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
    preview = spawn('npx', ['vite', 'preview', '--port', '1420', '--strictPort'], {
      stdio: 'ignore',
    });

    // One driver for the whole run. Spawning per session races on the port: every spec
    // after the first fails to bind and the run stalls.
    driver = spawn('tauri-driver', [], { stdio: 'ignore' });
    await new Promise((resolve) => setTimeout(resolve, 3000));
  },

  onComplete: () => {
    driver?.kill();
    preview?.kill();
    driver = null;
    preview = null;
    // KEEP_E2E_PROFILE leaves the profile and its log in place for diagnosis.
    if (!process.env.KEEP_E2E_PROFILE) {
      rmSync(E2E_PROFILE, { recursive: true, force: true });
    }
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
    const stub = test.title
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 80);
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    const dir = join('reports/screenshots', os);
    mkdirSync(dir, { recursive: true });
    const file = join(dir, `${stub}-${timestamp}.png`);

    // A failed capture must never fail the test it followed.
    const shot =
      os === 'darwin'
        ? spawnSync('screencapture', ['-x', '-o', file], { timeout: 10_000 })
        : spawnSync('import', ['-window', 'root', file], { timeout: 10_000 });
    if (shot.error) {
      console.warn(`screenshot failed for "${test.title}": ${shot.error.message}`);
    }
  },
};
