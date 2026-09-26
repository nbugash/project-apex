#!/usr/bin/env node
// Copies the signed-off design system from mockups/ into the interface asset tree.
//
// Constitution Principle I makes the mockup binding verbatim. Copying at build time means the
// only way the application can diverge is by editing the signed-off artifact — a visible,
// reviewable act. Hand-transcribing tokens would make drift silent, which is the failure the
// principle exists to prevent. An absent design system is therefore an error, never a
// degraded unstyled mode.
import { cp, mkdir, readdir, readFile, rm, writeFile, access } from 'node:fs/promises';
import { join } from 'node:path';

const MOCKUPS = 'mockups';
const DEST = 'client/ui/lib/ds';

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
// Layout tokens (F018, FR-009).
//
// The prototype's layout dimensions are NOT in the design system stylesheet — that defines
// colour, spacing, radius, shadow and font tokens only. They live in the prototype's own
// markup, some as `var(--vk-*, <value>)` fallbacks and some as literals on specific
// elements.
//
// Editing the design system to add them would modify a signed-off artifact, which the
// design fidelity principle forbids. Hand-authoring them in application code would make the
// application the source of truth for a value the prototype owns. So they are extracted
// here, on every build, exactly as the design system itself is copied rather than
// transcribed: the only way to change one is to change the prototype.
//
// Written inside client/ui/lib/ds/ deliberately — lint:ds skips that directory, and a token file
// necessarily contains the raw pixel values the lint forbids everywhere else.
const PROTOTYPE = join(MOCKUPS, 'Apex IDE (standalone).html');

// The six dimensions below are not literals in the markup. The prototype ships three
// density presets in a `DENS` table and applies one at runtime by setting the custom
// properties on the document element. The `var(--vk-*, <value>)` fallbacks scattered
// through the markup encode the `default` preset and are never the values displayed,
// because the script always runs — reading them produced a token file that disagreed with
// every screen the stakeholders signed off on.
//
// So the preset is resolved the way the prototype resolves it: read the declared default
// density from the component's own prop schema, then read that preset out of `DENS`.
const DENSITY_TOKENS = {
  line: { token: '--vk-line', what: 'editor line height' },
  row: { token: '--vk-row', what: 'tree row height' },
  fs: { token: '--vk-fs', what: 'interface font size' },
  code: { token: '--vk-code', what: 'code font size' },
  tool: { token: '--vk-tool', what: 'tool window width' },
  dock: { token: '--vk-dock', what: 'dock height' },
};

// The prototype declares its three semantic hues once, as module-level constants its terminal
// transcript, diff gutter, squiggles, minimap and VCS counts all read. That single declaration is
// the anchor; see the three entries in the table below.
const HUES = /const ERR='(#[0-9a-fA-F]{6})',\s*WARN='(#[0-9a-fA-F]{6})',\s*OK='(#[0-9a-fA-F]{6})'/;

// Literals on identifiable elements. These are genuine constants in the prototype — they
// do not vary with density — so they are measured where they are written.
const LITERAL_TOKENS = [
  {
    token: '--vk-chrome-height',
    re: /data-screen-label=\\?"Chrome\\?"[^>]*?height:(\d+px)/,
    what: 'chrome header height',
  },
  {
    token: '--vk-rail-width',
    re: /<nav style=\\?"flex:none;width:(\d+px)/,
    what: 'activity rail width',
  },
  {
    // The tree's per-level indent lives in the prototype's view model rather than in a style
    // attribute: `pad:(10+d*13)+'px'`. Read rather than retyped, for the same reason as every
    // other dimension here — a literal in a component is a second source of truth for a value
    // the prototype owns.
    token: '--vk-tree-indent',
    re: /pad:\(\d+\+d\*(\d+)\)\+'px'/,
    what: 'file tree per-level indent',
    unit: 'px',
  },
  {
    token: '--vk-tree-pad-left',
    re: /pad:\((\d+)\+d\*\d+\)\+'px'/,
    what: 'file tree base left padding',
    unit: 'px',
  },
  {
    // The three semantic hues, read from the prototype's own declaration of them. One regex
    // spanning all three rather than three matching a bare hex string each: a lone `#d4736a`
    // occurs wherever the prototype draws something red, so three independent patterns would
    // each anchor on whichever occurrence came first and would keep matching after the
    // declaration they are supposed to track had changed. Spanning the declaration means a
    // change to it breaks all three loudly, which is the same reason the surfaces below
    // anchor on structure.
    //
    // Sixteen ANSI colours, three of them defined here. The other thirteen are the terminal
    // library's own and are not invented (A-TERMPALETTE).
    token: '--vk-term-ansi-red',
    re: HUES,
    group: 1,
    what: 'terminal ANSI red (prototype ERR)',
  },
  {
    token: '--vk-term-ansi-yellow',
    re: HUES,
    group: 2,
    what: 'terminal ANSI yellow (prototype WARN)',
  },
  {
    token: '--vk-term-ansi-green',
    re: HUES,
    group: 3,
    what: 'terminal ANSI green (prototype OK)',
  },
  {
    // The tab's state dot. The prototype draws it as a filled circle for unsaved local
    // changes; F004 reuses the same footprint as a ring for changed-on-host, so the two are
    // distinguishable by shape. Extracted rather than retyped for the usual reason: a literal
    // in a component is a second source of truth for a value the prototype owns.
    token: '--vk-tab-dot',
    re: /width:(\d+px);height:\d+px;border-radius:50%;background:\{\{ t\.dirty \}\}/,
    what: 'tab state dot size',
  },
];

const prototypeMarkup = await readFile(PROTOTYPE, 'utf8');
const extracted = [];
const missing = [];

// The prop schema is HTML-entity encoded inside the standalone artifact.
const densityDefault =
  /options(?:&quot;|")\s*:\s*\[(?:&quot;|")compact(?:&quot;|")\s*,\s*(?:&quot;|")default(?:&quot;|")\s*,\s*(?:&quot;|")roomy(?:&quot;|")\]\s*,\s*(?:&quot;|")default(?:&quot;|")\s*:\s*(?:&quot;|")(\w+)(?:&quot;|")/.exec(
    prototypeMarkup,
  );
const densTable = /DENS\s*=\s*\{(.+?)\};/s.exec(prototypeMarkup);

if (!densityDefault) missing.push('the declared default density (prop schema)');
if (!densTable) missing.push('the DENS density table');

if (densityDefault && densTable) {
  const preset = new RegExp(`${densityDefault[1]}\\s*:\\s*\\{([^}]*)\\}`).exec(densTable[1]);
  if (!preset) {
    missing.push(`the "${densityDefault[1]}" preset inside DENS`);
  } else {
    const values = Object.fromEntries(
      preset[1]
        .split(',')
        .map((pair) => pair.split(':').map((x) => x.trim()))
        .filter(([k, v]) => k && v),
    );
    for (const [key, { token, what }] of Object.entries(DENSITY_TOKENS)) {
      if (values[key] === undefined)
        missing.push(`${token} (${what}, DENS.${densityDefault[1]}.${key})`);
      else
        extracted.push({
          token,
          value: `${values[key]}px`,
          what: `${what}, density ${densityDefault[1]}`,
        });
    }
  }
}

for (const { token, re, what, unit, group = 1 } of LITERAL_TOKENS) {
  const m = re.exec(prototypeMarkup);
  if (m) extracted.push({ token, value: unit ? `${m[group]}${unit}` : m[group], what });
  else missing.push(`${token} (${what})`);
}

// The chrome surfaces carry a further set of dimensions the design system does not define:
// button sizes, icon sizes, the rail's active mark, the tool window header's metrics. They
// are the prototype's, so they are read from it rather than retyped into a component, where
// they would be literals the adherence lint rightly rejects.
//
// Each surface is located once by a structural anchor, and its dimensions are read out of
// the style strings that follow. Anchoring on structure rather than on fifteen independent
// value patterns means a prototype change breaks one anchor loudly instead of silently
// matching some other element that happens to share a number.
const STYLE_ATTR = /style=\\?"([^"\\]*)/g;

function stylesAfter(anchor, count) {
  const at = anchor.exec(prototypeMarkup);
  if (!at) return null;
  const rest = prototypeMarkup.slice(at.index + at[0].length);
  const found = [];
  STYLE_ATTR.lastIndex = 0;
  let m;
  while ((m = STYLE_ATTR.exec(rest)) !== null && found.length < count) found.push(m[1]);
  return found.length === count ? found : null;
}

const decl = (style, prop) => {
  const m = new RegExp(`(?:^|;)\\s*${prop}\\s*:\\s*([^;]+)`).exec(style);
  return m ? m[1].trim() : null;
};

const SURFACES = [
  {
    name: 'chrome header',
    anchor: /data-screen-label=\\?"Chrome\\?"/,
    count: 1,
    read: [
      [0, 'height', '--vk-chrome-height', 'chrome header height'],
      [0, 'gap', '--vk-chrome-gap', 'chrome header gap'],
      [0, 'padding', '--vk-chrome-pad', 'chrome header padding'],
    ],
  },
  {
    name: 'activity rail',
    anchor: /<nav style=/,
    count: 1,
    // The anchor stops before `style=` so the rail's own attribute is the first match.
    offsetAnchor: /<nav (?=style=)/,
    read: [
      [0, 'width', '--vk-rail-width', 'activity rail width'],
      [0, 'padding', '--vk-rail-pad', 'activity rail padding'],
      [0, 'gap', '--vk-rail-gap', 'activity rail gap'],
    ],
  },
  {
    name: 'dock tabs',
    // The strip that carries Terminal, Debug, Problems and Resources. Anchored on the loop that
    // emits them rather than on the strip itself, so a prototype change breaks here loudly
    // instead of silently matching another flex row with the same numbers.
    //
    // Two styles follow: the button, and the badge inside it. The strip's own height sits on the
    // element *before* the loop, so it is read from the container anchor below.
    anchor: /<sc-for list=\\?"\{\{ dockTabs \}\}\\?"/,
    count: 3,
    read: [
      [0, 'gap', '--vk-dock-tab-gap', 'dock tab gap'],
      [0, 'padding', '--vk-dock-tab-pad', 'dock tab padding'],
      [0, 'font-size', '--vk-dock-tab-fs', 'dock tab font size'],
      [1, 'font-size', '--vk-dock-tab-icon-fs', 'dock tab icon font size'],
      [2, 'font-size', '--vk-dock-tab-badge-fs', 'dock tab badge font size'],
    ],
  },
  {
    name: 'dock tab strip',
    // The container, immediately before the loop. Its height is the prototype's to own; a
    // component writing it would be the application asserting a value read off a screenshot.
    //
    // The prototype has **two** strips with this flex shape -- the editor tabs above the
    // document area and the dock tabs below it -- differing only in which edge carries the
    // hairline. `inset 0 1px 0` is the dock's (the line is on top); the editor's is
    // `inset 0 -1px 0`. Anchoring on the shadow rather than on the height keeps the anchor
    // independent of the value being read, so a changed height is extracted rather than
    // silently missed.
    anchor: /box-shadow:inset 0 1px 0 color-mix\(in srgb,var\(--color-text\) 9%/,
    offsetAnchor: /<div (?=style=\\?"flex:none;display:flex;align-items:stretch;height:\d+px;background:var\(--color-surface\);box-shadow:inset 0 1px 0)/,
    count: 1,
    read: [[0, 'height', '--vk-dock-tabs-height', 'dock tab strip height']],
  },
  {
    name: 'file tree row',
    // Anchored on the loop that emits the rows, so a prototype change breaks here loudly
    // rather than silently matching some other element with the same numbers.
    anchor: /<sc-for list=\\?"\{\{ tree \}\}\\?"/,
    count: 4,
    read: [
      [0, 'gap', '--vk-tree-gap', 'file tree row gap'],
      [0, 'padding-right', '--vk-tree-pad-right', 'file tree row right padding'],
      [1, 'font-size', '--vk-tree-icon', 'file tree icon size'],
      [3, 'font-size', '--vk-tree-vcs-size', 'file tree vcs marker size'],
    ],
  },
  {
    name: 'terminal panel',
    // The `isTerminal` branch, which is the one structural marker the terminal surface has.
    // Five styles follow it: the transcript container, one transcript row, the prompt row, the
    // prompt itself, and the block cursor.
    //
    // 12.5px is a **third** font size, distinct from `--vk-fs` and `--vk-code`, which are both
    // 13.5px. A component writing `font-size: 12.5px` would be the application asserting a
    // value the prototype owns, which is the exact failure this file exists to prevent.
    anchor: /<sc-if value=\\?"\{\{ isTerminal \}\}\\?"/,
    count: 5,
    read: [
      [0, 'font-size', '--vk-term-fs', 'terminal font size'],
      [0, 'line-height', '--vk-term-line-height', 'terminal line height'],
      [0, 'padding', '--vk-term-pad', 'terminal padding'],
      [2, 'gap', '--vk-term-prompt-gap', 'terminal prompt row gap'],
      [4, 'width', '--vk-term-cursor-w', 'terminal cursor width'],
      [4, 'height', '--vk-term-cursor-h', 'terminal cursor height'],
      [4, 'animation', '--vk-term-cursor-blink', 'terminal cursor blink'],
    ],
  },
  {
    name: 'rail destination button',
    anchor: /<sc-for list=\\?"\{\{ rail \}\}\\?"/,
    // Six styles: the destination button and its icon and mark, the flex spacer, then the
    // rail's trailing collapse toggle and its icon — which is a size of its own, not the
    // destination icon size.
    count: 6,
    read: [
      [0, 'width', '--vk-rail-button', 'rail button size'],
      [1, 'font-size', '--vk-rail-icon', 'rail icon size'],
      [2, 'left', '--vk-rail-mark-left', 'rail active mark offset'],
      [2, 'top', '--vk-rail-mark-top', 'rail active mark top'],
      [2, 'height', '--vk-rail-mark-height', 'rail active mark height'],
      [4, 'width', '--vk-rail-toggle', 'rail collapse toggle size'],
      [5, 'font-size', '--vk-rail-toggle-icon', 'rail collapse toggle icon size'],
    ],
  },
  {
    name: 'chrome header controls',
    anchor: /data-screen-label=\\?"Chrome\\?"/,
    count: 22,
    read: [
      [1, 'gap', '--vk-chrome-mark-gap', 'product mark gap'],
      [2, 'width', '--vk-chrome-mark', 'product mark size'],
      [2, 'box-shadow', '--vk-chrome-mark-glow', 'product mark glow'],
      [3, 'font-size', '--vk-chrome-wordmark-size', 'wordmark size'],
      [3, 'letter-spacing', '--vk-chrome-wordmark-tracking', 'wordmark tracking'],
      [4, 'gap', '--vk-chrome-switcher-gap', 'project switcher gap'],
      [4, 'padding', '--vk-chrome-switcher-pad', 'project switcher padding'],
      [5, 'font-size', '--vk-chrome-switcher-icon', 'project switcher icon size'],
      [6, 'font-size', '--vk-chrome-caret', 'project switcher caret size'],
      [7, 'height', '--vk-chrome-divider-height', 'chrome divider height'],
      [8, 'gap', '--vk-chrome-run-gap', 'run group gap'],
      [9, 'gap', '--vk-chrome-runcfg-gap', 'run configuration gap'],
      [9, 'font-size', '--vk-chrome-runcfg-size', 'run configuration size'],
      [10, 'font-size', '--vk-chrome-runcfg-icon', 'run configuration icon size'],
      [11, 'font-size', '--vk-chrome-runcfg-caret', 'run configuration caret size'],
      [12, 'width', '--vk-chrome-iconbutton', 'chrome icon button size'],
      [13, 'font-size', '--vk-chrome-run-icon', 'run icon size'],
      [15, 'font-size', '--vk-chrome-debug-icon', 'debug icon size'],
      [16, 'max-width', '--vk-chrome-omni-max', 'omnibox maximum width'],
      [16, 'gap', '--vk-chrome-omni-gap', 'omnibox gap'],
      [16, 'height', '--vk-chrome-omni-height', 'omnibox height'],
      [16, 'padding', '--vk-chrome-omni-pad', 'omnibox padding'],
      [16, 'font-size', '--vk-chrome-omni-size', 'omnibox text size'],
      [17, 'font-size', '--vk-chrome-omni-icon', 'omnibox icon size'],
      [19, 'font-family', '--vk-mono', 'monospace family (the design system defines none)'],
      [19, 'font-size', '--vk-chrome-kbd-size', 'keyboard hint size'],
      [19, 'padding', '--vk-chrome-kbd-pad', 'keyboard hint padding'],
      [20, 'gap', '--vk-chrome-pill-gap', 'chrome pill gap'],
      [20, 'font-size', '--vk-chrome-pill-size', 'chrome pill text size'],
      [20, 'padding', '--vk-chrome-pill-pad', 'chrome pill padding'],
      [21, 'font-size', '--vk-chrome-pill-icon', 'chrome pill icon size'],
    ],
  },
  {
    name: 'status bar',
    anchor: /<footer (?=style=)/,
    count: 3,
    read: [
      [0, 'height', '--vk-status-height', 'status bar height'],
      [0, 'gap', '--vk-status-gap', 'status bar gap'],
      [0, 'padding', '--vk-status-pad', 'status bar padding'],
      [0, 'font-size', '--vk-status-size', 'status bar text size'],
      [2, 'font-size', '--vk-status-icon', 'status bar icon size'],
    ],
  },
  {
    name: 'tool window header',
    anchor: /data-screen-label=\\?"Tool window\\?"/,
    count: 7,
    read: [
      [1, 'height', '--vk-tool-header-height', 'tool window header height'],
      [1, 'gap', '--vk-tool-header-gap', 'tool window header gap'],
      [1, 'padding', '--vk-tool-header-pad', 'tool window header padding'],
      [2, 'font-size', '--vk-tool-label-size', 'tool window label size'],
      [2, 'letter-spacing', '--vk-tool-label-tracking', 'tool window label tracking'],
      [4, 'font-size', '--vk-tool-meta-size', 'tool window meta size'],
      [5, 'width', '--vk-tool-button', 'tool window header button size'],
      [5, 'border-radius', '--vk-tool-button-radius', 'tool window header button radius'],
      [6, 'font-size', '--vk-tool-button-icon', 'tool window header button icon size'],
    ],
  },
];

for (const surface of SURFACES) {
  const styles = stylesAfter(surface.offsetAnchor ?? surface.anchor, surface.count);
  if (!styles) {
    missing.push(`the ${surface.name} surface (anchor no longer matches)`);
    continue;
  }
  for (const [index, prop, token, what] of surface.read) {
    const value = decl(styles[index], prop);
    if (value === null) missing.push(`${token} (${what}, ${prop} of ${surface.name})`);
    else extracted.push({ token, value, what });
  }
}

// Two surfaces name the same dimension; a disagreement means one anchor drifted.
const byToken = new Map();
for (const e of extracted) {
  const prior = byToken.get(e.token);
  if (prior && prior.value !== e.value) {
    missing.push(`${e.token} read twice with different values ("${prior.value}" and "${e.value}")`);
  }
  byToken.set(e.token, e);
}
extracted.length = 0;
extracted.push(...byToken.values());

if (missing.length > 0) {
  console.error('\nds:sync — layout dimensions not found in the prototype:');
  for (const m of missing) console.error(`  ${m}`);
  console.error(
    '\nThe prototype changed shape and the extraction no longer matches. Fix the pattern\n' +
      'rather than hard-coding the value: a hand-written dimension is exactly the drift the\n' +
      'design fidelity principle exists to prevent.',
  );
  process.exit(1);
}

// The primary monospace family, on its own.
//
// `--vk-mono` is the prototype's whole stack and ends in the `monospace` generic, which matches
// every character -- so nothing appended after it is ever reached. A terminal needs to append:
// it renders whatever a program emits, including Powerline and Nerd Font glyphs in the private
// use area that no text font carries, and those are the characters a developer's shell prompt is
// built from. Splitting the first family out lets the terminal build a stack around the
// prototype's choice rather than restating it, which is what keeps the value the prototype's.
//
// Derived, not invented: if the prototype changes its monospace family, this changes with it.
const monoStack = extracted.find(({ token }) => token === '--vk-mono');
if (monoStack) {
  extracted.push({
    token: '--vk-mono-primary',
    value: monoStack.value.split(',')[0].trim(),
    what: 'monospace family, first entry only, for stacks that need to append',
  });
}

const tokenCss = [
  '/* GENERATED by scripts/ds-sync.mjs from the signed-off prototype. Do not edit.',
  ' *',
  ' * These dimensions are not part of the design system stylesheet; they live in the',
  ' * prototype markup. Changing one means changing the prototype.',
  ' */',
  ':root {',
  ...extracted.map(({ token, value, what }) => `  ${token}: ${value}; /* ${what} */`),
  '}',
  '',
].join('\n');

await writeFile(join(DEST, 'layout-tokens.css'), tokenCss);

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

// The layout tokens generated above are as real as the stylesheet's own. Leaving them out
// made this check report a design gap for every prototype dimension a component used
// correctly — which would have taught the first reader to disbelieve it.
for (const { token } of extracted) defined.add(token);

async function* sources(dir) {
  for (const e of await readdirAsync(dir, { withFileTypes: true })) {
    const full = join(dir, e.name);
    if (full.startsWith(DEST)) continue; // the system defines them; it does not consume them
    if (e.isDirectory()) yield* sources(full);
    else if (['.svelte', '.css', '.ts'].includes(extname(e.name))) yield full;
  }
}

const gaps = [];
for await (const file of sources('client/ui')) {
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
    `${defined.size} design tokens, ${extracted.length} layout tokens extracted from the ` +
    `prototype, no undefined token references.`,
);
