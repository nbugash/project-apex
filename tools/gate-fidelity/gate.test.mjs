// Self-tests for the fidelity gate (T043, T044 — SC-006, FR-015).
//
// A gate nobody tests is a gate that passes. This one has a specific way of failing
// silently: every check it performs could be inverted, or short-circuited, and the output
// would still read "pass" — which is the shape of an adherence lint this project has
// already been bitten by once.
//
// Alterations are injected through fixtures. Editing tracked source to prove the gate
// notices would mean the proof depends on remembering to undo it, and a half-undone proof
// is a failing build for the next person.
//
//   node --test tools/gate-fidelity/gate.test.mjs
//
// The cases here need no rendered window: they drive the comparison and its preconditions
// directly. The end-to-end proof that the gate passes on an unmodified build is
// `npm run gate:fidelity` itself, which CI runs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  compareGeometry,
  comparePixels,
  loadBaselineGeometry,
  GateUnrunnable,
} from './compare.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const REAL_GEOMETRY = join(HERE, 'reference/geometry.json');

const withTempDir = async (fn) => {
  const dir = await mkdtemp(join(tmpdir(), 'apex-gate-'));
  try {
    return await fn(dir);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
};

const approved = async () => JSON.parse(await readFile(REAL_GEOMETRY, 'utf8'));

test('passes when the rendering matches the derived geometry', async () => {
  const { surfaces } = await approved();
  assert.deepEqual(compareGeometry(surfaces, structuredClone(surfaces)), []);
});

test('passes when a surface is off by less than the tolerance', async () => {
  const { surfaces } = await approved();
  const actual = structuredClone(surfaces);
  actual['activity rail'].width += 2; // exactly the tolerance, not beyond it
  assert.deepEqual(compareGeometry(surfaces, actual), []);
});

test('fails, and names the surface, when a dimension is altered beyond the tolerance', async () => {
  const { surfaces } = await approved();
  const actual = structuredClone(surfaces);
  actual['activity rail'].width += 9;

  const failures = compareGeometry(surfaces, actual);
  assert.equal(failures.length, 1);
  // SC-007: naming the surface is the point. "something differs" sends the reader hunting.
  assert.match(failures[0], /activity rail/);
  assert.match(failures[0], /width/);
});

test('fails when a surface moves, not only when it resizes', async () => {
  const { surfaces } = await approved();
  const actual = structuredClone(surfaces);
  actual['tool window'].x += 20;

  const failures = compareGeometry(surfaces, actual);
  assert.equal(failures.length, 1);
  assert.match(failures[0], /tool window/);
  assert.match(failures[0], /\bx\b/);
});

test('fails when a surface is missing from the rendering entirely', async () => {
  const { surfaces } = await approved();
  const actual = structuredClone(surfaces);
  delete actual['chrome header'];

  const failures = compareGeometry(surfaces, actual);
  assert.equal(failures.length, 1);
  assert.match(failures[0], /chrome header/);
});

test('errors rather than passing when the geometry is absent', async () => {
  await withTempDir(async (dir) => {
    await assert.rejects(
      () => loadBaselineGeometry(join(dir, 'nope.json')),
      (e) => e instanceof GateUnrunnable && /missing/.test(e.message),
    );
  });
});

test('errors rather than passing when the geometry is unreadable', async () => {
  await withTempDir(async (dir) => {
    const path = join(dir, 'geometry.json');
    await writeFile(path, '{ this is not json');
    await assert.rejects(
      () => loadBaselineGeometry(path),
      (e) => e instanceof GateUnrunnable && /unreadable/.test(e.message),
    );
  });
});

test('errors rather than passing when the geometry has no surfaces', async () => {
  await withTempDir(async (dir) => {
    const path = join(dir, 'geometry.json');
    await writeFile(path, JSON.stringify({ reference: { width: 1200, height: 800 } }));
    await assert.rejects(
      () => loadBaselineGeometry(path),
      (e) => e instanceof GateUnrunnable && /no surfaces/.test(e.message),
    );
  });
});

test('errors rather than passing when the geometry was derived at another size', async () => {
  await withTempDir(async (dir) => {
    const path = join(dir, 'geometry.json');
    const real = await approved();
    await writeFile(
      path,
      JSON.stringify({ ...real, reference: { width: 1600, height: 1000 } }),
    );
    // Comparing a 1200-wide layout against a 1600-wide baseline would let real differences
    // through as though they were within tolerance.
    await assert.rejects(
      () => loadBaselineGeometry(path),
      (e) => e instanceof GateUnrunnable && /1600x1000/.test(e.message),
    );
  });
});

test('reports a difference, not an error, when the capture is a different size', async () => {
  await withTempDir(async (dir) => {
    const { execFileSync } = await import('node:child_process');
    const a = join(dir, 'a.png');
    const b = join(dir, 'b.png');
    execFileSync('convert', ['-size', '100x100', 'xc:black', a]);
    execFileSync('convert', ['-size', '120x100', 'xc:black', b]);

    const { differing } = comparePixels(a, b, join(dir, 'diff.png'));
    assert.equal(differing, Number.POSITIVE_INFINITY);
  });
});

test('counts differing pixels and writes the difference artifact', async () => {
  await withTempDir(async (dir) => {
    const { execFileSync } = await import('node:child_process');
    const { existsSync } = await import('node:fs');
    const a = join(dir, 'a.png');
    const b = join(dir, 'b.png');
    const diff = join(dir, 'diff.png');
    execFileSync('convert', ['-size', '100x100', 'xc:black', a]);
    // A 10x10 white square: 100 differing pixels out of 10 000, or 1%.
    execFileSync('convert', [a, '-fill', 'white', '-draw', 'rectangle 0,0 9,9', b]);

    const { differing } = comparePixels(a, b, diff);
    assert.equal(differing, 100);
    // FR-014: the artifact is what makes a failure actionable.
    assert.ok(existsSync(diff), 'a difference image must be written');
  });
});

// The cases above exercise the comparison. This one exercises the whole gate: it runs
// compare.mjs as CI runs it, against a fixture whose expected geometry has been altered,
// and requires a non-zero exit. Without it, the gate could measure nothing at all and every
// test above would still pass.
//
// It needs a display, because it launches the shell. Under `xvfb-run` it runs; elsewhere it
// reports why it did not, rather than silently counting as a pass.
test(
  'the whole gate fails on an altered dimension',
  { skip: process.env.DISPLAY ? false : 'needs a display; run under xvfb-run' },
  async () => {
    const { execFile } = await import('node:child_process');
    const { promisify } = await import('node:util');
    const run = promisify(execFile);

    await withTempDir(async (dir) => {
      const real = await approved();
      const altered = structuredClone(real);
      altered.surfaces['activity rail'].width += 40;
      const geometry = join(dir, 'geometry.json');
      await writeFile(geometry, JSON.stringify(altered, null, 2));

      let exitCode = 0;
      let stderr = '';
      try {
        await run('node', [join(HERE, 'compare.mjs')], {
          env: { ...process.env, APEX_FIDELITY_GEOMETRY: geometry, APEX_FIDELITY_OUT: dir },
          timeout: 300_000,
        });
      } catch (e) {
        exitCode = e.code ?? 1;
        stderr = e.stderr ?? '';
      }

      // The gate's own output goes into the message. Asserting on a number alone and
      // discarding the diagnostic turned a CI failure into thirty seconds of silence and
      // an exit code, which had to be reproduced locally to learn anything.
      assert.equal(
        exitCode,
        1,
        `an altered dimension must fail the gate (1), not error it (2).\nGate output:\n${
          stderr.trim() || '(nothing on stderr)'
        }`,
      );
      assert.match(stderr, /activity rail/, 'the failure must name the surface');
    });
  },
);
