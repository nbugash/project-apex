#!/usr/bin/env node
// Design-system adherence for markup and stylesheets.
//
// The ESLint rules ported from `_adherence.oxlintrc.json` match JS `Literal` nodes. That
// worked in the source configuration because React styles ARE JS literals. In Svelte styles
// live in markup attributes and <style> blocks, where no Literal node exists — so the ported
// rules parse cleanly and catch nothing. Preserving a rule's text while losing its effect is
// still weakening it, which the Design System Compliance section forbids. This closes the gap.
import { readdir, readFile } from 'node:fs/promises';
import { join, extname } from 'node:path';

const ROOTS = ['src'];
const SKIP = ['src/lib/ds'];
const EXTS = new Set(['.svelte', '.css', '.html']);

// The design system's rule is "never hard-code a px value THE TOKENS ALREADY CARRY". Its
// --space-* scale runs 2.8px, 5.6px, 8.4px upward and carries no 1px or 2px, so hairline
// borders and the focus ring it prescribes itself ("outline: 2px solid var(--color-accent);
// outline-offset: 2px") cannot come from a token. Flagging them would be stricter than the
// signed-off system, and a lint stricter than its own rule gets disabled.
const UNTOKENISED_PX = new Set([1, 2]);

const CHECKS = [
  {
    // Any hex colour. The design system exposes every colour as a token.
    test: (line) => (/#[0-9a-fA-F]{3,8}\b/.test(line) ? true : false),
    message: 'Raw hex colour — use a design-system colour token via var(--color-*).',
  },
  {
    // Static pixel values only. Interpolated sizes (`${extent}px`) are a region's runtime
    // dimension — data rather than design — and cannot come from a static token.
    test: (line) => {
      const matches = [...line.matchAll(/\b(\d+(?:\.\d+)?)px\b/g)];
      return matches.some((m) => !UNTOKENISED_PX.has(Number(m[1])));
    },
    message: 'Raw px value — use a design-system spacing token via var(--space-*).',
  },
  {
    // Capture the whole declaration value: a lookahead after \s* backtracks to zero width
    // and passes on ` var(...)`, which silently disables the rule.
    test: (line) => {
      const m = /font-family\s*:\s*([^;]+)/i.exec(line);
      return m ? !m[1].includes('var(') : false;
    },
    message: 'Hard-coded font family — use var(--font-heading) or var(--font-body).',
  },
];

async function* walk(dir) {
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (SKIP.some((s) => p.startsWith(s))) continue;
    if (e.isDirectory()) yield* walk(p);
    else if (EXTS.has(extname(e.name))) yield p;
  }
}

let failures = 0;
for (const root of ROOTS) {
  for await (const file of walk(root)) {
    const lines = (await readFile(file, 'utf8')).split('\n');
    lines.forEach((line, i) => {
      if (line.trimStart().startsWith('/*') || line.trimStart().startsWith('*')) return;
      if (line.includes('${')) return; // dynamic value, see note above
      for (const { test, message } of CHECKS) {
        if (test(line)) {
          console.error(`${file}:${i + 1}  ${message}\n    ${line.trim()}`);
          failures++;
        }
      }
    });
  }
}

if (failures > 0) {
  console.error(`\nlint:ds — ${failures} design-system violation(s).`);
  process.exit(1);
}
console.log('lint:ds — no design-system violations.');
