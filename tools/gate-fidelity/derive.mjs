#!/usr/bin/env node
// Derive the fidelity baseline's expected surface geometry FROM THE PROTOTYPE (T040).
//
// This is the part that makes the gate a fidelity check rather than a self-consistency
// check. Capturing the expected geometry from our own build would lock in whatever we
// happened to implement — including any drift already present — and thereafter catch only
// *future* drift, while reporting green against a design it had never actually matched.
//
// So the numbers come from rendering the signed-off prototype and measuring it, exactly as
// a reviewer would with a ruler, and the result is committed as reference/geometry.json.
//
// This script is NOT part of the CI gate. Deriving needs a browser engine to render the
// prototype, which the application's own toolchain has no other use for. It is run by hand
// when the prototype changes, and reference/derivation.md records when it last was.
//
//   node tools/gate-fidelity/derive.mjs [--playwright <path to playwright module>]
//
// The rendering engine is passed in rather than depended on, so this repository does not
// carry a browser download for a step it runs a handful of times.
import { writeFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const PROTOTYPE = 'file://' + join(HERE, '../../mockups/Apex IDE (standalone).html');

/** The reference size from the specification's Assumptions. A baseline is only meaningful
 *  at a stated size, because the flexible regions absorb everything else. */
export const REFERENCE = { width: 1200, height: 800 };

/** The surfaces SC-001 holds to the prototype, and how to find each one in both documents.
 *  The prototype has no class names — its markup is generated — so it is addressed by
 *  element, while our shell is addressed by the class the component declares. */
export const SURFACES = [
  { name: 'chrome header', prototype: 'header', shell: 'header.chrome' },
  { name: 'activity rail', prototype: 'nav', shell: 'nav.rail' },
  { name: 'tool window', prototype: 'aside', shell: 'aside.tool-window' },
];

function playwrightPath() {
  const flag = process.argv.indexOf('--playwright');
  if (flag !== -1 && process.argv[flag + 1]) return process.argv[flag + 1];
  return 'playwright';
}

function chromiumPath() {
  const flag = process.argv.indexOf('--chromium');
  return flag !== -1 ? process.argv[flag + 1] : undefined;
}

async function main() {
  let chromium;
  try {
    ({ chromium } = await import(playwrightPath()));
  } catch (e) {
    console.error(
      'derive: could not load a browser engine.\n' +
        '  Pass one explicitly:  node tools/gate-fidelity/derive.mjs --playwright <path>\n' +
        `  Underlying error: ${e.message}`,
    );
    process.exit(2);
  }

  const browser = await chromium.launch({ executablePath: chromiumPath() });
  const page = await browser.newPage({ viewport: REFERENCE, deviceScaleFactor: 1 });
  await page.goto(PROTOTYPE);
  // The prototype applies its density preset from script on mount; measuring before that
  // lands would record the CSS fallback values, which are a different preset entirely.
  await page.waitForTimeout(2500);

  const measured = await page.evaluate((surfaces) => {
    const out = {};
    for (const s of surfaces) {
      const el = document.querySelector(s.prototype);
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
  }, SURFACES);

  const missing = Object.entries(measured).filter(([, v]) => v === null);
  if (missing.length > 0) {
    console.error(
      'derive: these surfaces were not found in the prototype:\n' +
        missing.map(([k]) => `  ${k}`).join('\n') +
        '\nThe prototype changed shape. Fix the selector rather than the expected numbers.',
    );
    await browser.close();
    process.exit(1);
  }

  await browser.close();

  const geometry = {
    derivedFrom: 'mockups/Apex IDE (standalone).html',
    derivedOn: new Date().toISOString().slice(0, 10),
    reference: REFERENCE,
    surfaces: measured,
  };
  const out = join(HERE, 'reference/geometry.json');
  await writeFile(out, JSON.stringify(geometry, null, 2) + '\n');
  console.log(`derive: wrote ${out}`);
  for (const [name, r] of Object.entries(measured)) {
    console.log(`  ${name}: ${r.width}x${r.height} at ${r.x},${r.y}`);
  }
}

// Only when run directly. compare.mjs and update.mjs import SURFACES and REFERENCE from
// here, and an unguarded call would launch a browser on every gate run.
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  await main();
}
