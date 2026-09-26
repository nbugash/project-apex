/**
 * The expected cell grids SC-002 compares against.
 *
 * **Every grid here is written by hand from the escape sequence beside it.** None was captured
 * from a run. A grid recorded from the implementation under test is that implementation agreeing
 * with itself, which is not a test at all — and the same objection applies to comparing the panel
 * against the same terminal library rendering headlessly, which is why no local terminal is
 * consulted either.
 *
 * Each entry carries its input bytes so a reviewer can check the grid rather than trust it. That
 * is the point of the format: if a row below is wrong, the sequence above it is enough to say so
 * without running anything.
 *
 * The inventory exists because SC-002 reads "for each exercised sequence" and, with no list, is
 * satisfiable with one sequence — which measures the harness's ambition rather than the renderer.
 * quickstart §2 names these five as the minimum.
 */

import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** A cell's expected appearance. `undefined` means "not asserted", never "default". */
export interface Cell {
  /** The character. A space means the cell is blank, which is distinct from not asserted. */
  char: string;
  /** The CSS colour the panel must resolve the foreground to, if this cell asserts one. */
  fg?: string;
  bg?: string;
  bold?: boolean;
}

export interface Grid {
  /** What a failure message calls this. */
  name: string;
  /** Why the sequence is in the inventory at all. */
  why: string;
  /** The exact bytes written to the task's output, escapes and all. */
  input: string;
  cols: number;
  rows: number;
  /**
   * The rows that are asserted, keyed by zero-based row index. A row left out is not asserted;
   * a row present is asserted in full for the columns it lists.
   */
  expect: Record<number, Cell[]>;
  /** Where the cursor must be afterwards, zero-based, when the sequence moves it deliberately. */
  cursor?: { row: number; col: number };
}

const ESC = '\u001b';

/**
 * The three hues the design system defines, read from the file `ds-sync` generates.
 *
 * Read rather than restated. A literal here would be this test carrying a second copy of a value
 * the prototype owns -- which is the exact duplication `ds-sync` exists to prevent, and which the
 * adherence lint rightly refuses. SC-002 asserts the rendered colour equals the **extracted**
 * value, so the extraction is the source and the panel is what is under test.
 *
 * That the extracted value is itself correct is a different claim, guarded where it belongs: by
 * the anchor in `scripts/ds-sync.mjs`, which fails loudly if the prototype's declaration changes.
 *
 * Unlike the colours, the characters and positions below **are** hand-written. Those are claims
 * about the renderer and nothing else in the repository holds them.
 */
function readHues(): { red: string; yellow: string; green: string } {
  const css = readFileSync(join(process.cwd(), 'client/ui/lib/ds/layout-tokens.css'), 'utf8');
  const read = (token: string): string => {
    const found = new RegExp(`${token}:\\s*([^;]+);`).exec(css);
    if (!found?.[1]) {
      throw new Error(`${token} is not in layout-tokens.css; run npm run ds:sync`);
    }
    return found[1].trim();
  };
  return {
    red: read('--vk-term-ansi-red'),
    yellow: read('--vk-term-ansi-yellow'),
    green: read('--vk-term-ansi-green'),
  };
}

export const HUES = readHues();

/** Blank cells, for padding a row out to the width being asserted. */
const blanks = (n: number): Cell[] => Array.from({ length: n }, () => ({ char: ' ' }));

/** A row of plain characters, no colour asserted. */
const plain = (text: string): Cell[] => [...text].map((char) => ({ char }));

export const GRIDS: Grid[] = [
  {
    name: 'SGR colour',
    why:
      'Covers the three hues the design system defines and one slot the library supplies, which ' +
      'is what makes A-TERMPALETTE checkable from the rendered side rather than only from ' +
      'palette.ts. Without the library slot, a build that invented thirteen colours would pass.',
    // Red "ERR", green "OK", yellow "WARN", then blue — a slot with no design token — then reset.
    input:
      `${ESC}[31mERR${ESC}[0m ` +
      `${ESC}[32mOK${ESC}[0m ` +
      `${ESC}[33mWARN${ESC}[0m ` +
      `${ESC}[34mINFO${ESC}[0m`,
    cols: 20,
    rows: 2,
    expect: {
      0: [
        { char: 'E', fg: HUES.red },
        { char: 'R', fg: HUES.red },
        { char: 'R', fg: HUES.red },
        { char: ' ' },
        { char: 'O', fg: HUES.green },
        { char: 'K', fg: HUES.green },
        { char: ' ' },
        { char: 'W', fg: HUES.yellow },
        { char: 'A', fg: HUES.yellow },
        { char: 'R', fg: HUES.yellow },
        { char: 'N', fg: HUES.yellow },
        { char: ' ' },
        // Blue is asserted as a character only. Its colour is the library's, and pinning a
        // literal here would be this repository deciding a value A-TERMPALETTE says it does not
        // own. The spec asserts instead that it differs from all three hues and from the
        // foreground, which is the claim that actually matters.
        { char: 'I' },
        { char: 'N' },
        { char: 'F' },
        { char: 'O' },
        ...blanks(4),
      ],
    },
  },

  {
    name: 'cursor addressing (CUP)',
    why:
      'A character must land where no sequential write would put it. An implementation that ' +
      'appends and ignores addressing produces the same characters in the wrong places, and a ' +
      'test asserting only on text would pass.',
    // Write a marker on the first row, then jump to row 3 column 5 -- the sequence counts from
    // one, the grid below from zero -- and write a character there. Nothing is written between,
    // so a renderer that ignored the jump would leave row 3 empty and put X after "top".
    input: `top${ESC}[3;5HX`,
    cols: 10,
    rows: 4,
    expect: {
      0: [...plain('top'), ...blanks(7)],
      // Row index 2 is the sequence's row 3. Column index 4 is its column 5.
      2: [...blanks(4), { char: 'X' }, ...blanks(5)],
    },
    cursor: { row: 2, col: 5 },
  },

  {
    name: 'erase in line (EL)',
    why:
      'Cells that held characters must read as blank afterwards. An implementation that moves ' +
      'the cursor but does not erase leaves the old text visible, which is indistinguishable ' +
      'from correct until something shorter is written over something longer.',
    // Write nine characters, return to column 4, erase to end of line.
    input: `ABCDEFGHI${ESC}[1;4H${ESC}[K`,
    cols: 10,
    rows: 2,
    expect: {
      // ABC survives; everything from column 4 onward is blank, not stale.
      0: [...plain('ABC'), ...blanks(7)],
    },
    cursor: { row: 0, col: 3 },
  },

  {
    name: 'carriage-return redraw',
    why:
      'The progress-bar case, and the one an append-only implementation gets wrong silently. A ' +
      'shorter write after \\r overwrites only the cells it reaches, so the tail of the first ' +
      'line is still there. A grid that assumes the row was cleared is asserting a terminal ' +
      'nobody has.',
    // Twelve characters, carriage return, then five written over the front of them.
    input: 'building 87%\rdone!',
    cols: 14,
    rows: 2,
    expect: {
      // "done!" covers "build"; "ing 87%" remains, because \r returns the cursor and erases
      // nothing. This is the assertion the whole entry exists for.
      0: [...plain('done!'), ...plain('ing 87%'), ...blanks(2)],
    },
  },

  {
    name: 'scroll region (DECSTBM)',
    why:
      'Rows inside the region move and rows outside it do not. An implementation that scrolls ' +
      'the whole screen produces a plausible-looking result in which the header has vanished, ' +
      'which is what a region exists to prevent.',
    // A header on row 1, then rows 2..4 set as the scroll region, then four lines written into
    // it — one more than fits, so the region scrolls exactly once.
    input: `HEADER\r\n` + `${ESC}[2;4r` + `${ESC}[2;1Hone\r\n` + `two\r\n` + `three\r\n` + `four`,
    cols: 8,
    rows: 5,
    expect: {
      // Outside the region: untouched by the scroll.
      0: [...plain('HEADER'), ...blanks(2)],
      // Inside: "one" scrolled off the top, leaving two/three/four.
      1: [...plain('two'), ...blanks(5)],
      2: [...plain('three'), ...blanks(3)],
      3: [...plain('four'), ...blanks(4)],
    },
  },
];

/** Every escape byte in the inventory, for the assertion that none of them is rendered. */
export const ESCAPE_BYTE = ESC;
