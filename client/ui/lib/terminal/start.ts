/**
 * Starting a terminal, once.
 *
 * VS Code and IntelliJ both do this: the tool window has no shell until you look at it, and
 * looking at it a second time shows you the one you already have. Starting eagerly would run a
 * login shell -- with the developer's full profile, and whatever that does -- for somebody who
 * never opened the panel.
 *
 * # Why "once" is a module fact and not a component one
 *
 * The panel is mounted and unmounted as the dock opens and closes, so a guard held in component
 * state resets on the second open and starts a second shell. This lives beside the panels it
 * guards, which outlive the view for the same reason (FR-026).
 */

import { taskSink } from './sink';
import { terminals } from './terminals.svelte';

/// What a bare terminal runs.
///
/// The developer's own login shell, because that is what every other terminal gives them and a
/// different one silently drops their aliases, prompt and path. An argv vector, not a command
/// line: §7.3 scopes this as process execution and not a shell, so nothing interposes `sh -c`
/// and nothing has to reason about quoting.
const DEFAULT_SHELL = ['/bin/bash', '-l'];

/// §4.8's default, used until a mounted panel measures itself.
const UNMEASURED = { cols: 80, rows: 24 };

/// In flight or already started. Not a boolean on a panel, because the question is "has this
/// client started its terminal" and the answer has to survive the panel being unmounted.
let started: string | null = null;
let starting = false;

/// Whether a terminal exists for this client to show.
export function hasTerminal(): boolean {
  return started !== null && terminals.has(started);
}

/// The identity of the running terminal, or null.
export function currentTerminal(): string | null {
  return hasTerminal() ? started : null;
}

/// Forget the terminal, so the next reveal starts a fresh one. For tests, and for a task the
/// engine has released.
export function forgetTerminal(): void {
  started = null;
  starting = false;
}

/**
 * Start a terminal if this client has none, and show it either way.
 *
 * Returns the task identity, or null if the start was refused — no engine, no workspace, or a
 * command the engine would not run. A null is not retried on its own: the caller is a click, and
 * a person who clicks again is the retry.
 */
export async function revealTerminal(): Promise<string | null> {
  if (hasTerminal()) {
    terminals.show(started as string);
    return started;
  }
  // A second click while the first start is in flight must not spawn a second shell. The engine
  // would refuse it (-32010, one process per identity), but only because the identity collides;
  // two clicks a second apart would mint two identities and get two shells.
  if (starting) return null;
  starting = true;
  try {
    // The client chooses the identity (FR-001). Time-based rather than counted, so two windows
    // of the same application cannot mint the same one.
    const id = `term-${Date.now()}`;
    const panel = terminals.show(id);
    const size = panel.hasTerminal ? panel.fit() : UNMEASURED;
    const ok = await taskSink().run(id, DEFAULT_SHELL, size.cols, size.rows);
    if (!ok) {
      // Released rather than left showing a terminal that will never fill. A panel for a task
      // that does not exist is the same lie this feature started with.
      terminals.release(id);
      return null;
    }
    started = id;
    return id;
  } finally {
    starting = false;
  }
}
