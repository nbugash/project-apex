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
import type { FitAddon } from '@xterm/addon-fit';

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
  #pending: string[] = [];
  #terminal: Terminal | null = null;
  #fit: FitAddon | null = null;

  constructor(taskId: string) {
    this.taskId = taskId;
  }

  /// Everything received while unattached, as one string. The attached panel's own buffer is the
  /// source once `attach` has run, and this is empty.
  buffered(): string {
    return this.#pending.join('');
  }

  get attached(): boolean {
    return this.#terminal !== null;
  }

  /// Accept output, whether or not anything is on screen to show it.
  write(data: string): void {
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
    const [{ Terminal: XTerm }, { FitAddon: Fit }] = await Promise.all([
      import('@xterm/xterm'),
      import('@xterm/addon-fit'),
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
    this.#terminal?.dispose();
    this.#terminal = null;
    this.#fit = null;
  }
}

/// The set of panels, keyed by task id.
export class Terminals {
  /// An array rather than a map: the count is the number of tasks a person is watching, so the
  /// linear lookup is cheaper than the reactivity wrapper a keyed collection would need.
  panels = $state<TerminalPanel[]>([]);

  /// The panel for `taskId`, created on first ask. Asking twice yields the same panel, which is
  /// the identity FR-026 rests on.
  panel(taskId: string): TerminalPanel {
    const existing = this.panels.find((p) => p.taskId === taskId);
    if (existing) return existing;
    const created = new TerminalPanel(taskId);
    this.panels.push(created);
    return created;
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
  }
}

export const terminals = new Terminals();
