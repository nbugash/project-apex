// T017 — US1. The offline state and the "requires the engine" wording carry an icon and a word,
// resolve to design-system tokens, and stay apart in greyscale (FR-002, FR-004).
//
// **Compared by luminance, not by hue**, following `git-marker.test.ts` and
// `rail-greyscale.spec.ts`. A design carrying the whole distinction in colour passes a hue
// comparison perfectly and is unreadable for roughly one developer in twelve.
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { PRESENTATION, present } from '../../client/ui/lib/statusbar/presentation';
import { describeOutcome } from '../../client/ui/lib/editor/ending';
import type { WriteOutcome } from '../../client/ui/lib/editor/buffers.svelte';

const SYSTEM = readFileSync('client/ui/lib/ds/system/styles.css', 'utf8');
const STATUS_BAR = readFileSync('client/ui/lib/statusbar/StatusBar.svelte', 'utf8');
const FILE_TREE = readFileSync('client/ui/lib/workspace/FileTree.svelte', 'utf8');

/** The hex a token resolves to in the design system, so the test reads what ships. */
function valueOf(token: string): string {
  const m = new RegExp(`${token}\\s*:\\s*(#[0-9a-fA-F]{3,8})`).exec(SYSTEM);
  if (!m) throw new Error(`${token} is not a design-system token`);
  return m[1]!;
}

/** Relative luminance, which is what survives a greyscale rendering. */
function luminance(hex: string): number {
  const h = hex.slice(1);
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

describe('offline presentation', () => {
  it('states offline with a word as well as an icon', () => {
    // The connection half. F003 built this and F012 depends on it rather than adding a second
    // offline indicator, which is why it is asserted here instead of assumed.
    const offline = present('disconnected');
    expect(offline.label).toBe('Offline');
    expect(offline.icon.length).toBeGreaterThan(0);
    // Every state, not only this one: a state added later without a label would encode itself in
    // colour alone, and this is the assertion that notices.
    for (const [state, presented] of Object.entries(PRESENTATION)) {
      expect(presented.label.length, `${state} has no word`).toBeGreaterThan(0);
      expect(presented.icon.length, `${state} has no icon`).toBeGreaterThan(0);
    }
  });

  it('says work is held locally with a word as well as an icon', () => {
    // Read out of the component, so the assertion is about what ships rather than about a copy.
    // A copy cannot notice a change in the original, which is the mistake `presentation.ts`
    // records the status-bar test making before it was extracted.
    expect(STATUS_BAR).toContain('held locally');
    expect(STATUS_BAR).toMatch(/class="held"[\s\S]{0,200}<i class="ph ph-[a-z-]+"/);
  });

  it('marks an unlisted folder unavailable with a word as well as an icon', () => {
    expect(FILE_TREE).toContain('Not available offline');
    expect(FILE_TREE).toMatch(/class="unavailable"[\s\S]{0,200}<i class="ph ph-[a-z-]+"/);
  });

  it('distinguishes held-locally and unavailable from ordinary text in greyscale', () => {
    // Both new surfaces use the neutral ramp deliberately -- work held locally and a folder
    // nobody has listed are ordinary consequences of being offline, not faults -- so the
    // assertion is that they are separated from the body text by luminance, not that they are
    // coloured like a warning.
    const held = /\.held\s*\{[^}]*color:\s*var\((--color-[a-z0-9-]+)\)/.exec(STATUS_BAR);
    const unavailable = /\.unavailable\s*\{[^}]*color:\s*var\((--color-[a-z0-9-]+)\)/.exec(
      FILE_TREE,
    );
    expect(held, 'the held-locally indicator must take its colour from a token').not.toBeNull();
    expect(unavailable, 'the unavailable mark must take its colour from a token').not.toBeNull();

    const body = luminance(valueOf('--color-neutral-100'));
    for (const [what, token] of [
      ['held locally', held![1]!],
      ['unavailable', unavailable![1]!],
    ] as const) {
      const value = luminance(valueOf(token));
      expect(
        Math.abs(value - body),
        `${what} is indistinguishable from body text in greyscale`,
      ).toBeGreaterThan(8);
    }
  });

  it('never encodes either state in colour alone', () => {
    // The property both of the above serve, stated once: removing every colour from the two new
    // surfaces must leave a reader able to tell what they say. A word is what guarantees that,
    // and this asserts the word is inside the element that carries the colour rather than
    // somewhere else in the file.
    expect(STATUS_BAR).toMatch(/class="held"[\s\S]{0,300}held locally/);
    expect(FILE_TREE).toMatch(/class="unavailable"[\s\S]{0,300}Not available offline/);
  });
});

describe('path search presentation', () => {
  const SEARCH = readFileSync('client/ui/lib/workspace/PathSearch.svelte', 'utf8');

  it('says the results may not be everything, in words', () => {
    // FR-007. The caveat is the requirement: a results list with no caveat presents a partial
    // cache as the whole repository, and a caveat carried by styling alone is no caveat.
    expect(SEARCH).toContain('Showing cached results');
    expect(SEARCH).toMatch(/data-testid="path-search-partial"[\s\S]{0,300}Showing cached results/);
  });

  it('shows the caveat only when the answer is incomplete', () => {
    // Guarded on `!result.complete`, not on the connection state. A connected search that hit its
    // limit is also incomplete, and deriving the caveat from the connection would miss it.
    expect(SEARCH).toMatch(/\{#if !result\.complete\}/);
  });

  it('takes every colour from a design-system token', () => {
    // Principle I, and the surface is not in the prototype, so this is the assertion that keeps a
    // designer able to move it. `lint-ds.mjs` catches a raw hex; it does not catch `rgb()` or a
    // named colour, so the rule is asserted here rather than left to the lint's reach.
    const colours = SEARCH.match(/(?:^|[^-\w])color:\s*([^;]+);/g) ?? [];
    expect(colours.length).toBeGreaterThan(0);
    for (const decl of colours) {
      // `inherit` and `currentColor` are not hard-coded values -- they defer to whatever the
      // design system already put on an ancestor, which is the point of the rule rather than an
      // exception to it.
      if (/inherit|currentColor/i.test(decl)) continue;
      expect(decl.trim()).toMatch(/var\(--/);
    }
  });
});

describe('save outcomes', () => {
  it('reports a held save as a success that does not claim the host has it', () => {
    // FR-012. Two things a developer needs: the work is safe, and the host does not have it yet.
    // "Saved" alone is the confusion §11.2 forbids; "Not saved" would be false.
    const held = describeOutcome({ kind: 'heldLocally' });
    expect(held.tone).toBe('ok');
    expect(held.title).not.toMatch(/not saved/i);
    expect(`${held.title} ${held.detail ?? ''}`).toMatch(/host|connection/i);
    expect(held.offersReload).toBe(false);
  });

  it('keeps a held save and an unreachable host as different stories', () => {
    // The two must not read as each other, which is what F006 built `unreachable` to say and what
    // F012 must not blur. A held save is a success with the work on this machine; an unreachable
    // host is a failure with the work still in the buffer.
    //
    // T031b asked for `unreachable` to be reworded on the grounds that an offline save is now held.
    // It is -- but by `file_write` routing to the retainer *before* `EditFile` is reached, so
    // `unreachable` never describes the offline case, and a retain that fails returns `Refused`.
    // F006's copy was right and its tests were right to reject the rewrite.
    const held = describeOutcome({ kind: 'heldLocally' });
    const unreachable = describeOutcome({ kind: 'unreachable' });
    expect(held.tone).toBe('ok');
    expect(unreachable.tone).toBe('error');
    expect(unreachable.title).toMatch(/could not be reached/i);
    expect(`${held.title} ${held.detail ?? ''}`).not.toMatch(/not saved/i);
  });

  it('labels every variant of the union, so a sixth fails here as well as in the compiler', () => {
    // Over every variant rather than the new one. `describeOutcome`'s switch would fail to build
    // without a case, but only because its return type is exhaustive -- a `default` added later
    // would silence that, and this is what would still notice.
    const all: WriteOutcome[] = [
      { kind: 'written', sha256: 'a'.repeat(64) },
      { kind: 'heldLocally' },
      { kind: 'conflict' },
      { kind: 'refused', message: 'no' },
      { kind: 'unreachable' },
    ];
    for (const outcome of all) {
      const label = describeOutcome(outcome);
      expect(label.title.length, `${outcome.kind} has no title`).toBeGreaterThan(0);
      expect(['ok', 'warning', 'error']).toContain(label.tone);
    }
  });

  it('treats a held save as a success in the save path, not a failure', () => {
    // T031a's half, and the silent one. `buffers.svelte.ts` routes every outcome that is not
    // `written` to `failed()` unless something says otherwise, and the `if/else` compiles either
    // way -- so this reads the source rather than trusting the compiler.
    const BUFFERS = readFileSync('client/ui/lib/editor/buffers.svelte.ts', 'utf8');
    expect(BUFFERS).toMatch(/outcome\.kind === 'heldLocally'\) b\.held\(\)/);
    // And `held()` keeps the base: reconciliation merges against the content the host confirmed,
    // and moving the base here would lose it.
    expect(BUFFERS).toMatch(/held\(\): void \{[\s\S]{0,160}this\.dirty = false;/);
    expect(BUFFERS).not.toMatch(/held\(\): void \{[\s\S]{0,160}this\.base =/);
  });
});
