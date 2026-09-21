#!/usr/bin/env node
// Copies the signed-off design system from mockups/ into the interface asset tree.
//
// Constitution Principle I makes the mockup binding verbatim. Copying at build time means the
// only way the application can diverge is by editing the signed-off artifact — a visible,
// reviewable act. Hand-transcribing tokens would make drift silent, which is the failure the
// principle exists to prevent. An absent design system is therefore an error, never a
// degraded unstyled mode.
import { cp, mkdir, readdir, rm, access } from 'node:fs/promises';
import { join } from 'node:path';

const MOCKUPS = 'mockups';
const DEST = 'src/lib/ds';

async function exists(p) {
  try { await access(p); return true; } catch { return false; }
}

async function findDesignSystem() {
  const dsRoot = join(MOCKUPS, '_ds');
  if (!(await exists(dsRoot))) {
    throw new Error(`No design system at ${dsRoot}. The signed-off mockup is required; refusing to build unstyled.`);
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

console.log(`ds:sync — copied ${bundle} plus fonts and icons into ${DEST}`);
