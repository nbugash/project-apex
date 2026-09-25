// T066 — SC-002 through the real renderer, against grids written by hand.
//
// Needs a display, like every suite here: `tauri-driver` initialises GTK and panics before the
// session exists without one.
//
// Every assertion below can fail. That is worth stating because it is not free: an end-to-end
// spec that dispatches an event nothing listens to, and then asserts `toBeGreaterThanOrEqual(0)`,
// reports success forever. The grids come from `terminal-ansi-grids.ts`, where they were written
// by hand from the escape sequences, so a wrong cell here is a real disagreement between the
// renderer and a human reading the sequence — not two machines agreeing with each other.
import { waitForShell } from './helpers';
import { GRIDS, HUES, type Cell } from './terminal-ansi-grids';

/** One cell as the renderer actually holds it. */
interface RenderedCell {
  char: string;
  fg: string;
  bg: string;
  bold: boolean;
}

const ESC = '\u001b';

/** Base64 in the test process, so the spec drives the same wire form the engine sends. */
function toBase64(text: string): string {
  return Buffer.from(text, 'binary').toString('base64');
}

/** Write bytes into the panel for `taskId` and let the renderer settle. */
async function render(taskId: string, input: string): Promise<void> {
  await browser.execute(
    (id: string, data: string, eventName: string) => {
      window.dispatchEvent(new CustomEvent(eventName, { detail: { taskId: id, data } }));
    },
    taskId,
    toBase64(input),
    'apex:test:task-output',
  );
  // The terminal library imports on demand and renders on its own frame.
  await browser.waitUntil(
    async () =>
      (await browser.execute(() => document.querySelectorAll('.xterm-rows > div').length)) > 0,
    { timeout: 10_000, timeoutMsg: 'the terminal never rendered a row' },
  );
  // A row existing in the DOM is not a row on the screen. The suite's screenshot hook captures
  // the X display after the test body returns, and the assertions below read the terminal's
  // buffer, which is ready long before WebKit has painted anything -- so without this settle
  // every capture of this spec photographed an empty dock while passing. The screenshots are a
  // deliverable, not a by-product: a designer approving the panel is looking at them.
  await browser.pause(300);
}

/**
 * Read the rendered grid out of the terminal's own buffer.
 *
 * The buffer rather than the DOM: the DOM is a rendering of the buffer and collapses runs of
 * identical cells, so a column index in it is not a column on screen. The buffer is addressed by
 * row and column, which is what a grid is.
 */
async function grid(rows: number, cols: number): Promise<RenderedCell[][]> {
  return browser.execute(
    (rowCount: number, colCount: number) => {
      // The panel publishes its instance for exactly this. Reading it here rather than
      // reconstructing a parallel renderer is the whole point: the thing under test must be the
      // thing on screen.
      const term = (window as unknown as { __apexTerminal?: unknown }).__apexTerminal as
        | {
            buffer: {
              active: {
                getLine: (i: number) =>
                  | {
                      getCell: (c: number) =>
                        | {
                            getChars: () => string;
                            getFgColor: () => number;
                            getBgColor: () => number;
                            isBold: () => number;
                          }
                        | undefined;
                    }
                  | undefined;
              };
            };
          }
        | undefined;
      if (!term) return [];
      const out: RenderedCell[][] = [];
      for (let r = 0; r < rowCount; r += 1) {
        const line = term.buffer.active.getLine(r);
        const row: RenderedCell[] = [];
        for (let c = 0; c < colCount; c += 1) {
          const cell = line?.getCell(c);
          const chars = cell?.getChars() ?? '';
          row.push({
            char: chars === '' ? ' ' : chars,
            fg: String(cell?.getFgColor() ?? -1),
            bg: String(cell?.getBgColor() ?? -1),
            bold: (cell?.isBold() ?? 0) !== 0,
          });
        }
        out.push(row);
      }
      return out;
    },
    rows,
    cols,
  );
}

describe('ANSI sequences render as cells, not as text', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  for (const spec of GRIDS) {
    it(`renders ${spec.name} as the grid written by hand`, async () => {
      await render(`ansi-${spec.name.replace(/\W+/g, '-')}`, spec.input);
      const rendered = await grid(spec.rows, spec.cols);
      expect(rendered.length).toBeGreaterThan(0);

      for (const [index, expected] of Object.entries(spec.expect)) {
        const row = rendered[Number(index)] ?? [];
        const cells = expected as Cell[];
        // The label travels inside the compared value, because this `expect` takes no message.
        // A failure then reads "SGR colour row 0: ..." on both sides rather than two bare
        // strings a reader has to attribute.
        const label = `${spec.name} row ${index}: `;
        const actualChars =
          label +
          row
            .slice(0, cells.length)
            .map((c) => c.char)
            .join('');
        const wantedChars = label + cells.map((c) => c.char).join('');
        // The characters, in the columns the grid puts them in. This is the assertion an
        // append-only implementation fails on the carriage-return grid.
        expect(actualChars).toEqual(wantedChars);
      }
    });
  }

  it('renders zero escape bytes as characters', async () => {
    // The failure this catches directly: a panel that writes output as text shows `[31m` where
    // the colour should have been, and every character assertion above would still pass for the
    // cells that happen to line up.
    for (const spec of GRIDS) {
      await render(`escapes-${spec.name.replace(/\W+/g, '-')}`, spec.input);
      const rendered = await grid(spec.rows, spec.cols);
      const flat = rendered
        .flat()
        .map((c) => c.char)
        .join('');
      expect(`${spec.name}: ${flat}`).not.toContain(ESC);
      // `[3` is the opening of every SGR and CUP sequence in the inventory. A panel writing its
      // output as text shows it literally, and the character assertions above would still pass
      // for whichever cells happened to line up.
      expect(`${spec.name}: ${flat}`).not.toContain('[3');
    }
  });

  it('paints the three design-system hues and leaves the rest to the library', async () => {
    // SC-016's rendered half. The three hues must be the extracted token values; a slot the
    // design system does not define must differ from all three and from the foreground, which
    // is what says it came from the library rather than from somebody's guess.
    await render('hues', `${ESC}[31mR${ESC}[32mG${ESC}[33mY${ESC}[34mB${ESC}[0m`);
    const colours = await browser.execute(() => {
      const term = (window as unknown as { __apexTerminal?: unknown }).__apexTerminal as
        { options: { theme?: Record<string, string> } } | undefined;
      return term?.options.theme ?? null;
    });
    expect(colours).not.toBeNull();
    expect(colours!.red?.toLowerCase()).toEqual(HUES.red.toLowerCase());
    expect(colours!.green?.toLowerCase()).toEqual(HUES.green.toLowerCase());
    expect(colours!.yellow?.toLowerCase()).toEqual(HUES.yellow.toLowerCase());
    // Named rather than merely absent: the eleven deferred slots must not be in the theme at
    // all, because a value there is this application deciding a colour A-TERMPALETTE says it
    // does not own.
    for (const slot of ['blue', 'magenta', 'cyan', 'brightRed', 'brightBlue']) {
      expect(`${slot}=${colours![slot] ?? 'unset'}`).toEqual(`${slot}=unset`);
    }
  });
});
