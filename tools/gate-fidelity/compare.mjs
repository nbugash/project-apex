#!/usr/bin/env node
// The fidelity gate (FR-012 … FR-015).
//
// Two judgements, because they catch different things:
//
//   Geometry, against numbers derived from the PROTOTYPE. This is the fidelity check. It
//   fails when a surface drifts from the approved design, and it would fail today if the
//   implementation had never matched in the first place.
//
//   Pixels, against a baseline image of THIS application, approved once its geometry was
//   reconciled against the prototype. This is the regression check. A pixel baseline taken
//   from the prototype itself could never pass — the prototype is a populated mock with a
//   file tree, an editor and a terminal, while the shell is mostly empty — so comparing
//   against it would produce a gate that is red from the first run and therefore ignored.
//
// Exit codes: 0 pass, 1 differences found, 2 the gate could not run.
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { withShell, measureSurfaces, captureWindow, REFERENCE } from './measure.mjs';
import { SURFACES } from './derive.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, '../..');
// Overridable so the gate's own tests can point it at a fixture — an altered dimension, a
// missing baseline — and exercise this exact code path rather than a reimplementation of
// it. A self-test that tests a copy of the logic proves nothing about the gate.
const GEOMETRY = process.env.APEX_FIDELITY_GEOMETRY ?? join(HERE, 'reference/geometry.json');
const BASELINE = process.env.APEX_FIDELITY_BASELINE ?? join(HERE, 'reference/baseline-1200x800.png');
const OUT_DIR = process.env.APEX_FIDELITY_OUT ?? join(REPO, 'reports/fidelity');

/** From the specification's Assumptions. Engineering thresholds, not design decisions. */
const POSITION_TOLERANCE_PX = 2;
const AREA_TOLERANCE = 0.005;

export class GateUnrunnable extends Error {}

export async function loadBaselineGeometry(path = GEOMETRY) {
  if (!existsSync(path)) {
    throw new GateUnrunnable(
      `the reference geometry is missing at ${path}\n` +
        '  Derive it from the prototype: node tools/gate-fidelity/derive.mjs',
    );
  }
  let parsed;
  try {
    parsed = JSON.parse(await readFile(path, 'utf8'));
  } catch (e) {
    throw new GateUnrunnable(`the reference geometry is unreadable: ${e.message}`);
  }
  if (!parsed?.surfaces || typeof parsed.surfaces !== 'object') {
    throw new GateUnrunnable('the reference geometry has no surfaces');
  }
  // A baseline captured at another size is not a baseline for this run. Passing it would
  // compare a 1200-wide layout against a 1600-wide one and call small differences fine.
  const ref = parsed.reference ?? {};
  if (ref.width !== REFERENCE.width || ref.height !== REFERENCE.height) {
    throw new GateUnrunnable(
      `the reference geometry was derived at ${ref.width}x${ref.height}, ` +
        `but this gate compares at ${REFERENCE.width}x${REFERENCE.height}`,
    );
  }
  return parsed;
}

export function compareGeometry(expected, actual) {
  const failures = [];
  for (const { name } of SURFACES) {
    const want = expected[name];
    const got = actual[name];
    if (!want) {
      failures.push(`${name}: no expected geometry in the reference`);
      continue;
    }
    if (!got) {
      failures.push(`${name}: not present in the rendered shell`);
      continue;
    }
    for (const axis of ['x', 'y', 'width', 'height']) {
      const delta = Math.abs(got[axis] - want[axis]);
      if (delta > POSITION_TOLERANCE_PX) {
        failures.push(
          `${name}: ${axis} is ${got[axis]}, expected ${want[axis]} ` +
            `(off by ${delta}px, tolerance ${POSITION_TOLERANCE_PX}px)`,
        );
      }
    }
  }
  return failures;
}

/** Differing pixel count via ImageMagick, which is already a dependency of the screenshot
 *  path. `compare -metric AE` writes the count to stderr and the difference image to disk,
 *  so one call satisfies both the area tolerance and the failure artifact. */
export function comparePixels(baseline, candidate, diffPath) {
  // Dimensions first, explicitly. ImageMagick does NOT refuse a size mismatch: it compares
  // the overlapping region and reports the count for that, so a capture of the wrong size
  // comes back as zero differing pixels — a pass. Every other check in this gate could be
  // correct and a resized window would still sail through.
  const sizeOf = (file) => {
    const r = spawnSync('identify', ['-format', '%wx%h', file], { encoding: 'utf8' });
    if (r.error) throw new GateUnrunnable(`could not run ImageMagick identify: ${r.error.message}`);
    return (r.stdout || '').trim();
  };
  const baselineSize = sizeOf(baseline);
  const candidateSize = sizeOf(candidate);
  if (baselineSize !== candidateSize) {
    return {
      differing: Number.POSITIVE_INFINITY,
      note: `the capture is ${candidateSize} and the baseline is ${baselineSize}`,
    };
  }

  const result = spawnSync(
    'compare',
    ['-metric', 'AE', '-fuzz', '2%', baseline, candidate, diffPath],
    { encoding: 'utf8' },
  );
  if (result.error) {
    throw new GateUnrunnable(`could not run ImageMagick compare: ${result.error.message}`);
  }
  const output = (result.stderr || '').trim();
  const differing = Number.parseFloat(output.split(/\s+/)[0]);
  if (!Number.isFinite(differing)) {
    throw new GateUnrunnable(`could not read a pixel count from ImageMagick: "${output}"`);
  }
  return { differing, note: null };
}

export async function main() {
  const expected = await loadBaselineGeometry();

  if (!existsSync(BASELINE)) {
    throw new GateUnrunnable(
      `the baseline image is missing at ${BASELINE}\n` +
        '  Approve one deliberately: npm run gate:fidelity:update',
    );
  }

  await mkdir(OUT_DIR, { recursive: true });
  const candidate = join(OUT_DIR, 'candidate.png');
  const diff = join(OUT_DIR, 'difference.png');

  const actual = await withShell(async (browser) => {
    const surfaces = await measureSurfaces(browser, SURFACES);
    captureWindow(candidate);
    return surfaces;
  });

  const geometryFailures = compareGeometry(expected.surfaces, actual);
  const { differing, note } = comparePixels(BASELINE, candidate, diff);

  const totalPixels = REFERENCE.width * REFERENCE.height;
  const ratio = differing / totalPixels;
  const pixelsFailed = ratio > AREA_TOLERANCE;

  await writeFile(
    join(OUT_DIR, 'result.json'),
    JSON.stringify(
      { reference: REFERENCE, expected: expected.surfaces, actual, differing, ratio },
      null,
      2,
    ) + '\n',
  );

  if (geometryFailures.length === 0 && !pixelsFailed) {
    console.log(
      `gate:fidelity — pass. Three surfaces within ${POSITION_TOLERANCE_PX}px of the ` +
        `prototype; ${(ratio * 100).toFixed(3)}% of pixels differ from the baseline.`,
    );
    return 0;
  }

  console.error('gate:fidelity — FAIL\n');
  if (geometryFailures.length > 0) {
    console.error('Surfaces differing from the prototype-derived geometry:');
    for (const f of geometryFailures) console.error(`  ${f}`);
    console.error('');
  }
  if (pixelsFailed) {
    const percent = Number.isFinite(ratio) ? `${(ratio * 100).toFixed(3)}%` : 'an unmeasurable share';
    console.error(
      `Rendering differs from the approved baseline: ${percent} of pixels ` +
        `(tolerance ${(AREA_TOLERANCE * 100).toFixed(1)}%).${note ? ` ${note}.` : ''}`,
    );
    console.error(`  difference image: ${diff}`);
    console.error(`  candidate:        ${candidate}`);
    console.error('');
  }
  console.error(
    'If the approved design changed, update the baseline deliberately:\n' +
      '  npm run gate:fidelity:update',
  );
  return 1;
}

// Only when run directly; the self-tests import the pieces above.
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    process.exit(await main());
  } catch (e) {
    // FR-015: an absent or unreadable baseline is an error, never a pass.
    console.error(`gate:fidelity — cannot run: ${e.message}`);
    process.exit(2);
  }
}
