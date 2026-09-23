// Shared measurement for the fidelity gate: launch the shell at the reference size, read
// the surface geometry out of it, and capture its pixels.
//
// Used by both compare.mjs and update.mjs, so the approved baseline and the thing judged
// against it are produced by identical steps. If they were captured differently, the gate
// would be measuring the difference between two capture paths rather than between two
// renderings.
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import { existsSync, rmSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { remote } from 'webdriverio';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, '../..');
// Workspace root, not the member — see the note in tests/e2e/wdio.conf.ts.
const BINARY = join(REPO, 'target/debug/apex-shell');
const PROFILE = join(REPO, '.gate-profile');

export const REFERENCE = { width: 1200, height: 800 };

/** The application's devUrl, verbatim. Probing 127.0.0.1 instead looked equivalent and was
 *  not: vite binds the name `localhost`, which on a machine that resolves it to ::1 leaves
 *  an IPv4 probe refused while the webview connects perfectly well. The gate waited out its
 *  full timeout against a server that was up the whole time. */
const PREVIEW_URL = 'http://localhost:1420';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Is something already answering where the bundle should be served?
 *
 *  An HTTP check rather than a socket bind. Binding 127.0.0.1 to test availability gave a
 *  false "free" against a server listening on ::1 — the same address-family mismatch that
 *  made the readiness probe fail. What matters is whether something answers, not which
 *  family it answers on. */
async function somethingIsServing() {
  try {
    await fetch(PREVIEW_URL, { signal: AbortSignal.timeout(1500) });
    return true;
  } catch {
    return false;
  }
}

/** Kill a child and everything it spawned. Requires the child to be detached. */
function killGroup(child) {
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

class PreviewUnservable extends Error {}

async function waitForPreview(log, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = 'no response';
  while (Date.now() < deadline) {
    try {
      const res = await fetch(PREVIEW_URL);
      if (res.ok) return;
      // A response means the server is up. Retrying will not change what it serves, so a
      // 404 is reported now rather than after thirty seconds of polling something that
      // was answering the whole time.
      throw new PreviewUnservable(
        `the preview server is running on port 1420 but returned HTTP ${res.status}.\n` +
          '  It has nothing to serve: the built bundle is missing. Run: npm run build',
      );
    } catch (e) {
      if (e instanceof PreviewUnservable) throw e;
      lastError = e.message;
    }
    // vite says so plainly when it cannot bind. Waiting out the timeout after that adds
    // thirty seconds and tells the reader nothing the first line had not already said.
    if (log.join('').includes('already in use')) {
      throw new PreviewUnservable(
        'the preview server could not bind port 1420 — something else holds it.\n' +
          `  Its output was:\n${log.join('').trim().replace(/^/gm, '    ')}`,
      );
    }
    await sleep(500);
  }
  // The server's own output, not just "it never came up". Without this the gate reports a
  // 30-second silence and the reader has to reproduce the whole job to learn why.
  const output = log.join('').trim();
  throw new Error(
    `the preview server never became reachable on port 1420 (last attempt: ${lastError}).\n` +
      (output
        ? `  Its output was:\n${output.replace(/^/gm, '    ')}`
        : '  It produced no output at all, which usually means it never started. ' +
          'A built bundle must exist: npm run build'),
  );
}

/** Launch the shell, hand it to `fn`, and tear everything down afterwards. */
export async function withShell(fn) {
  if (!existsSync(BINARY)) {
    throw new Error(
      `the shell binary is missing at ${BINARY}\n` +
        '  Build it first: cargo build --manifest-path client/core/Cargo.toml',
    );
  }

  // A debug build loads devUrl, so without a server on that port the webview renders blank
  // and the gate would compare two empty windows and pass.
  // The gate serves the bundle itself and refuses to borrow someone else's server. The
  // application's devUrl is fixed at 1420, so the gate cannot move to another port — and
  // a server it did not start may be serving an older build, which would make the
  // comparison judge the wrong pixels while reporting success.
  if (await somethingIsServing()) {
    throw new Error(
      'port 1420 is already in use, so the gate cannot serve the bundle it is meant to judge.\n' +
        '  Something else is listening — most likely a preview server left behind by an\n' +
        '  earlier run. Stop it and try again; the gate will not compare against a server\n' +
        '  it did not start, because that server may hold a different build.',
    );
  }

  // Output is captured rather than discarded, and drained as it arrives so a full pipe
  // cannot block the server. A silent failure here used to surface only as a timeout.
  const previewLog = [];
  const preview = spawn('npx', ['vite', 'preview', '--port', '1420', '--strictPort'], {
    cwd: REPO,
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
  });
  preview.stdout.on('data', (d) => previewLog.push(d.toString()));
  preview.stderr.on('data', (d) => previewLog.push(d.toString()));
  preview.on('error', (e) => previewLog.push(`could not spawn the preview server: ${e.message}\n`));

  rmSync(PROFILE, { recursive: true, force: true });
  mkdirSync(PROFILE, { recursive: true });
  process.env.APEX_DATA_DIR = PROFILE;

  // See tests/e2e/wdio.conf.ts: without this, window creation waits out a ~22 second
  // portal activation timeout on a machine with no desktop session.
  process.env.DBUS_SESSION_BUS_ADDRESS = '/dev/null';

  const driver = spawn('tauri-driver', [], { stdio: 'ignore', detached: true });
  let browser = null;
  try {
    await waitForPreview(previewLog);
    await sleep(3000); // tauri-driver needs its port before the first session

    browser = await remote({
      hostname: '127.0.0.1',
      port: 4444,
      path: '/',
      logLevel: 'error',
      capabilities: { 'tauri:options': { application: BINARY } },
    });

    await browser.$('.shell').waitForExist({ timeout: 30_000 });
    // A hidden window reports a placeholder size. Measuring it would produce a baseline of
    // a window nobody ever saw — which is exactly the defect that shipped in F000.
    await browser.waitUntil(async () => (await browser.execute(() => window.innerWidth)) > 400, {
      timeout: 30_000,
      timeoutMsg: 'the window never became visible',
    });

    await browser.setWindowRect(0, 0, REFERENCE.width, REFERENCE.height);
    await sleep(500);

    return await fn(browser);
  } finally {
    if (browser) await browser.deleteSession().catch(() => {});
    // Groups, not wrappers: npx spawns vite as a child, and killing npx alone leaves vite
    // holding port 1420 for whatever runs next.
    killGroup(driver);
    killGroup(preview);
    rmSync(PROFILE, { recursive: true, force: true });
  }
}

/** Read each surface's rect, in the viewport's coordinates, as the prototype was measured. */
export async function measureSurfaces(browser, surfaces) {
  return browser.execute((list) => {
    const out = {};
    for (const s of list) {
      const el = document.querySelector(s.shell);
      if (!el) {
        out[s.name] = null;
        continue;
      }
      const r = el.getBoundingClientRect();
      out[s.name] = {
        x: Math.round(r.x),
        y: Math.round(r.y),
        width: Math.round(r.width),
        height: Math.round(r.height),
      };
    }
    return out;
  }, surfaces);
}

/** Capture the shell window's pixels to `path`.
 *
 *  Not WebDriver's saveScreenshot: WebKitWebDriver leaves that request open until the
 *  command times out, which turns every capture into a minute of waiting and then a
 *  failure. Capturing at the X level is immediate and gets the same pixels.
 *
 *  And not `import -window <id>` either — that asks the X server for the window's own
 *  backing store, which a compositing-free Xvfb session declines with "Resource
 *  temporarily unavailable". Grabbing the root always works, so the window is located and
 *  cropped out of it instead. */
export function captureWindow(path) {
  // Located by geometry, not by name. The X window that carries the process name is the
  // 10x10 placeholder Tauri creates before the interface signals readiness; the window a
  // user actually sees is a descendant of it. Searching by name found the placeholder and
  // captured a hundred pixels of nothing.
  const tree = spawnSync('xwininfo', ['-root', '-tree'], { encoding: 'utf8' }).stdout ?? '';
  const candidates = [];
  for (const line of tree.split('\n')) {
    const m = /(\d+)x(\d+)\+-?\d+\+-?\d+\s+\+(-?\d+)\+(-?\d+)/.exec(line);
    if (!m) continue;
    const [, w, h, x, y] = m.map(Number);
    if (w >= REFERENCE.width && h >= REFERENCE.height) candidates.push({ w, h, x, y });
  }

  if (candidates.length === 0) {
    throw new Error(
      `no window at least ${REFERENCE.width}x${REFERENCE.height} is on the display.\n` +
        '  The gate needs a rendered window at the reference size; run it under a display\n' +
        '  with room for one, for example: xvfb-run -s "-screen 0 1400x900x24" …',
    );
  }

  // The tightest fit, so a root window covering the whole screen never wins over the
  // application's own.
  candidates.sort((a, b) => a.w * a.h - b.w * b.h);
  const win = candidates[0];

  const full = join(REPO, 'reports/fidelity/.root.png');
  mkdirSync(dirname(full), { recursive: true });
  execFileSync('import', ['-window', 'root', full]);
  execFileSync('convert', [
    full,
    '-crop',
    `${REFERENCE.width}x${REFERENCE.height}+${win.x}+${win.y}`,
    '+repage',
    path,
  ]);
  rmSync(full, { force: true });
}
