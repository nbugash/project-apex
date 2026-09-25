/// The terminal panels, one per task.
///
/// FR-026 is the whole point of this file: two tasks are two terminals, sharing nothing. A single
/// instance keyed by nothing would satisfy every positive claim about output arriving somewhere and
/// would still be wrong, so the set is keyed by task id and each entry owns its own buffer, its own
/// dimensions and its own scrollback.
///
/// Follows `client/ui/lib/workspace/tree.svelte.ts` as this project's rune-backed store precedent.
///
/// The terminal library is loaded by **dynamic import inside `attach`**, not at module scope. That
/// is deliberate and load-bearing twice over: it keeps a terminal out of the startup bundle for a
/// session that never runs a task, and it lets a panel be created, written to and asserted about
/// without a DOM -- which is what makes the isolation above testable at the unit level rather than
/// only through a driven interface.

import type { ITheme, Terminal } from '@xterm/xterm';
import { decodeBase64 } from './wire';
import type { FitAddon } from '@xterm/addon-fit';

/// Used only to render the panel's own text into the buffered form. Output is never decoded.
const ENCODER = new TextEncoder();

/// §4.8's defaults, used until a mounted panel measures itself.
export const DEFAULT_COLS = 80;
export const DEFAULT_ROWS = 24;

/// Lines kept above the viewport.
///
/// The library's own default is 1 000, which is too few to scroll back through a compile -- the
/// case this panel exists for. Stated here rather than accepted silently.
export const SCROLLBACK_LINES = 10_000;

/// One task's terminal.
export class TerminalPanel {
  readonly taskId: string;
  /// The dimensions last agreed with the engine, which are what `execution/resizePty` reports.
  cols = $state(DEFAULT_COLS);
  rows = $state(DEFAULT_ROWS);

  /// Output that arrived before this panel was mounted, in arrival order.
  ///
  /// A task's output does not wait for its dock tab to be visible, and dropping what arrives first
  /// would lose the beginning of every build -- the part that says what is being built. Held as
  /// plain data so that a panel which has never been attached still answers for what it received.
  #pending: Array<string | Uint8Array> = [];
  #terminal: Terminal | null = null;
  #fit: FitAddon | null = null;
  /// Whether this panel's task was given a terminal, which decides how an interrupt is sent.
  #terminalShape = true;
  #disposers: Array<() => void> = [];

  constructor(taskId: string, hasTerminal = true) {
    this.taskId = taskId;
    this.#terminalShape = hasTerminal;
  }

  /// Whether the task has a terminal (A-TASKSTREAM).
  ///
  /// The panel branches on it rather than asking the engine, because it is the shape **this
  /// client chose** when it started the task, and asking would be a round trip to be told
  /// something already known.
  get hasTerminal(): boolean {
    return this.#terminalShape;
  }

  /// Every keystroke the terminal reports, as the library gives it: a string, because that is
  /// what a key event is. It becomes bytes at the boundary, once (see `wire.ts`).
  onInput(handler: (data: string) => void): void {
    if (!this.#terminal) return;
    const sub = this.#terminal.onData(handler);
    this.#disposers.push(() => sub.dispose());
  }

  /// Every size the terminal settles on, so the engine can be told.
  onResize(handler: (cols: number, rows: number) => void): void {
    if (!this.#terminal) return;
    const sub = this.#terminal.onResize(({ cols, rows }) => {
      this.cols = cols;
      this.rows = rows;
      handler(cols, rows);
    });
    this.#disposers.push(() => sub.dispose());
  }

  /// Everything received while unattached, in arrival order. The attached panel's own buffer is
  /// the source once `attach` has run, and this is empty.
  ///
  /// Returns bytes rather than text, because output is bytes: a caller wanting a string has to ask
  /// for one, at which point the decode is its decision and not a silent one made here.
  buffered(): Uint8Array {
    const parts = this.#pending.map((p) => (typeof p === 'string' ? ENCODER.encode(p) : p));
    const total = parts.reduce((n, p) => n + p.length, 0);
    const out = new Uint8Array(total);
    let at = 0;
    for (const part of parts) {
      out.set(part, at);
      at += part.length;
    }
    return out;
  }

  get attached(): boolean {
    return this.#terminal !== null;
  }

  /// Accept output, whether or not anything is on screen to show it.
  ///
  /// Takes bytes or text. Output arrives as bytes and stays bytes; the string form is for the
  /// panel's own writing -- a reconnection summary, an ending -- which this application authored
  /// and therefore knows the encoding of.
  write(data: string | Uint8Array): void {
    if (this.#terminal) this.#terminal.write(data);
    else this.#pending.push(data);
  }

  /// Mount into `el` and replay whatever arrived first.
  ///
  /// `theme` is resolved by the caller rather than read here, because an `ITheme` takes colour
  /// strings and the tokens they come from live on the mounted element -- see `palette.ts`, which
  /// is the one place any of this application's terminal colours is decided.
  async attach(el: HTMLElement, theme: ITheme): Promise<void> {
    if (this.#terminal) return;
    // The stylesheet travels with the library, on the same terms and for the same reason. It is
    // not decoration: without it the rows are unpositioned and the hidden input the terminal uses
    // to receive keystrokes is drawn as a white box over the panel. A terminal whose buffer is
    // correct and whose pixels are not passes every assertion about cells and is still unusable,
    // which is the failure end-to-end tests exist for and which this one missed -- it reads the
    // buffer, and the buffer was right the whole time.
    const [{ Terminal: XTerm }, { FitAddon: Fit }] = await Promise.all([
      import('@xterm/xterm'),
      import('@xterm/addon-fit'),
      import('@xterm/xterm/css/xterm.css'),
    ]);
    const terminal = new XTerm({
      scrollback: SCROLLBACK_LINES,
      cursorStyle: 'block',
      cursorBlink: true,
      theme,
      cols: this.cols,
      rows: this.rows,
    });
    const fit = new Fit();
    terminal.loadAddon(fit);
    terminal.open(el);
    for (const chunk of this.#pending) terminal.write(chunk);
    this.#pending = [];
    this.#terminal = terminal;
    this.#fit = fit;
    this.fit();
    publishForAutomation(terminal);
  }

  /// Re-resolve the palette without rebuilding the terminal, so a theme change keeps the scrollback.
  retheme(theme: ITheme): void {
    if (this.#terminal) this.#terminal.options.theme = theme;
  }

  /// Set the dimensions directly, as the engine's own view of them.
  ///
  /// Separate from `fit`, which measures. This is the path a resize takes when the size is
  /// decided elsewhere -- `cols` and `rows` supplied at `runTask`, or a reattachment agreeing a
  /// size with a task that outlived the connection. Zero is ignored: some programs read a zero
  /// dimension as "no terminal", so forwarding one would change a task's behaviour rather than
  /// its layout.
  resize(cols: number, rows: number): void {
    if (cols <= 0 || rows <= 0) return;
    this.cols = cols;
    this.rows = rows;
    this.#terminal?.resize(cols, rows);
  }

  /// Size to the element, and report what that came to so the engine can be told.
  fit(): { cols: number; rows: number } {
    if (this.#fit && this.#terminal) {
      this.#fit.fit();
      this.cols = this.#terminal.cols;
      this.rows = this.#terminal.rows;
    }
    return { cols: this.cols, rows: this.rows };
  }

  /// Release the library instance. The panel stays usable: further output buffers again, which is
  /// what makes hiding a dock tab and showing it later lossless.
  detach(): void {
    for (const dispose of this.#disposers.splice(0)) dispose();
    this.#terminal?.dispose();
    this.#terminal = null;
    this.#fit = null;
  }
}

/// Expose the mounted terminal so the end-to-end suite can read its cell buffer.
///
/// The buffer is the only place a *grid* exists: the DOM is a rendering of it that collapses runs
/// of identical cells, so a column index in the DOM is not a column on screen. SC-002 is a claim
/// about rows and columns, so it has to be checked against the thing addressed by rows and
/// columns.
///
/// Published only under automation, like the harness that writes into it. A handle to the live
/// terminal is a handle to everything a person has typed into it.
function publishForAutomation(terminal: Terminal): void {
  if (!(import.meta.env.DEV || navigator.webdriver === true)) return;
  (window as unknown as { __apexTerminal?: Terminal }).__apexTerminal = terminal;
}

/// Deliver one `execution/onOutput` chunk to its panel.
///
/// The buffering FR-022 needs happens by construction rather than by logic here. Before a panel is
/// attached the chunks queue in arrival order; after it is attached the terminal library's own
/// parser holds a partial escape sequence or a partial multi-byte character until the rest of it
/// arrives. A chunk boundary is where the engine ended a frame and means nothing, so neither layer
/// is allowed to treat one as a delimiter.
export function applyChunk(panel: TerminalPanel, encoded: string): void {
  panel.write(decodeBase64(encoded));
}

/// The set of panels, keyed by task id.
export class Terminals {
  /// An array rather than a map: the count is the number of tasks a person is watching, so the
  /// linear lookup is cheaper than the reactivity wrapper a keyed collection would need.
  panels = $state<TerminalPanel[]>([]);

  /// The task whose terminal the dock is showing, or null for the idle prompt.
  ///
  /// One at a time, because the dock has one terminal region. The panels themselves are per task
  /// and keep buffering whether or not they are the one on screen (FR-026), so switching between
  /// them loses nothing -- which is what makes a single visible slot a presentation choice rather
  /// than a limit on how many tasks may run.
  active = $state<string | null>(null);

  /// The panel for `taskId`, created on first ask. Asking twice yields the same panel, which is
  /// the identity FR-026 rests on.
  panel(taskId: string, hasTerminal = true): TerminalPanel {
    const existing = this.panels.find((p) => p.taskId === taskId);
    if (existing) return existing;
    const created = new TerminalPanel(taskId, hasTerminal);
    this.panels.push(created);
    return created;
  }

  /// Make `taskId`'s panel the one the dock shows, creating it if this is the first sight of it.
  show(taskId: string, hasTerminal = true): TerminalPanel {
    const panel = this.panel(taskId, hasTerminal);
    this.active = taskId;
    return panel;
  }

  has(taskId: string): boolean {
    return this.panels.some((p) => p.taskId === taskId);
  }

  /// Forget one task's panel, leaving every other panel exactly as it was.
  release(taskId: string): void {
    const at = this.panels.findIndex((p) => p.taskId === taskId);
    const going = this.panels[at];
    if (!going) return;
    going.detach();
    this.panels.splice(at, 1);
    if (this.active === taskId) {
      // Fall back to whatever is left rather than to the idle prompt, so releasing one of several
      // running tasks does not look like every task ending.
      this.active = this.panels[0]?.taskId ?? null;
    }
  }
}

export const terminals = new Terminals();
