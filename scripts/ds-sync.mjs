#!/usr/bin/env node
// Copies the signed-off design system from mockups/ into the interface asset tree.
//
// Constitution Principle I makes the mockup binding verbatim. Copying at build time means the
// only way the application can diverge is by editing the signed-off artifact — a visible,
// reviewable act. Hand-transcribing tokens would make drift silent, which is the failure the
// principle exists to prevent. An absent design system is therefore an error, never a
// degraded unstyled mode.
import { cp, mkdir, readdir, readFile, rm, access } from 'node:fs/promises';
import { join } from 'node:path';

const MOCKUPS = 'mockups';
const DEST = 'src/lib/ds';

async function exists(p) {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

async function findDesignSystem() {
  const dsRoot = join(MOCKUPS, '_ds');
  if (!(await exists(dsRoot))) {
    throw new Error(
      `No design system at ${dsRoot}. The signed-off mockup is required; refusing to build unstyled.`,
    );
  }
  const entries = await readdir(dsRoot, { withFileTypes: true });
  const bundle = entries.find((e) => e.isDirectory());
  if (!bundle) throw new Error(`No design system bundle inside ${dsRoot}.`);
  return join(dsRoot, bundle.name);
}

const bundle = await findDesignSystem();
const stylesheet = join(bundle, 'styles.css');
if (!(await exists(stylesheet))) {
  throw new Error(`Design system bundle at ${bundle} has no styles.css.`);
}

await rm(DEST, { recursive: true, force: true });
await mkdir(DEST, { recursive: true });
await cp(bundle, join(DEST, 'system'), { recursive: true });

for (const asset of ['fonts', 'icons']) {
  const src = join(MOCKUPS, asset);
  if (!(await exists(src))) throw new Error(`Missing ${src}; the design system requires it.`);
  await cp(src, join(DEST, asset), { recursive: true });
}

// ---------------------------------------------------------------------------
// FR-022: where the approved design does not cover a surface the feature needs, the gap is
// resolved with the designer before that surface is built. A component reaching for a token
// the design system never defined IS that gap, and it fails silently at runtime — the
// property just resolves to nothing and the element renders unstyled. This turns it into a
// build failure at the moment it is introduced.
import { readdir as readdirAsync, readFile as readFileAsync } from 'node:fs/promises';
import { extname } from 'node:path';

const stylesheetText = await readFile(join(DEST, 'system/styles.css'), 'utf8');
const defined = new Set([...stylesheetText.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));

async function* sources(dir) {
  for (const e of await readdirAsync(dir, { withFileTypes: true })) {
    const full = join(dir, e.name);
    if (full.startsWith(DEST)) continue; // the system defines them; it does not consume them
    if (e.isDirectory()) yield* sources(full);
    else if (['.svelte', '.css', '.ts'].includes(extname(e.name))) yield full;
  }
}

const gaps = [];
for await (const file of sources('src')) {
  const text = await readFileAsync(file, 'utf8');
  text.split('\n').forEach((line, i) => {
    for (const m of line.matchAll(/var\(\s*(--[a-z0-9-]+)/g)) {
      if (!defined.has(m[1])) gaps.push(`${file}:${i + 1}  ${m[1]}`);
    }
  });
}

if (gaps.length > 0) {
  console.error('\nds:sync — components reference tokens the signed-off design does not define:');
  for (const g of gaps) console.error(`  ${g}`);
  console.error(
    '\nFR-022: resolve the gap with the designer and record the resolution before building\n' +
      'this surface. Do not improvise a value from an adjacent token.',
  );
  process.exit(1);
}

console.log(
  `ds:sync — copied ${bundle} plus fonts and icons into ${DEST}; ` +
    `${defined.size} tokens available, no undefined token references.`,
);
