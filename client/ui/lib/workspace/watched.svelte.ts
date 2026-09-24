/// What the client asks the engine to watch.
///
/// **Reasons, not conclusions.** The set sent is folder paths for expanded folders and *file*
/// paths for open tabs, and the engine derives the directories from it. If this file resolved
/// that itself and sent only directories, a folder holding an open file would arrive as one
/// path for two reasons, and unwatching on a collapse could not be told from unwatching on a
/// tab close — so collapsing a folder would silently stop reporting a file still open inside
/// it (FR-003c, FR-004, A-WATCHSCOPE).

/** The set, derived from what the developer has expanded and opened. */
export function watchedPaths(
  expandedFolders: readonly string[],
  openTabPaths: readonly string[],
): string[] {
  // Deduplicated, because the same path can be both — a folder open as a tab, or two tabs
  // beside each other. The engine counts reasons per directory; the client counts paths.
  return [...new Set([...expandedFolders, ...openTabPaths])].sort();
}

/** What changed between two sets, so only the difference is sent. */
export interface WatchDelta {
  add: string[];
  remove: string[];
}

export function delta(previous: readonly string[], next: readonly string[]): WatchDelta {
  const before = new Set(previous);
  const after = new Set(next);
  return {
    add: next.filter((p) => !before.has(p)),
    remove: previous.filter((p) => !after.has(p)),
  };
}

/// Coalesce rapid changes before asking.
///
/// Expanding three folders quickly is three renders and should be one request. Not a
/// correctness property — the set is idempotent and a redundant call changes nothing — but
/// watch establishment sits inside an interaction the developer initiated (§1.4), and three
/// round trips where one would do is three times the budget spent.
export class WatchRequester {
  #current: string[] = [];
  #timer: ReturnType<typeof setTimeout> | null = null;
  #send: (d: WatchDelta) => void;
  #debounceMs: number;

  constructor(send: (d: WatchDelta) => void, debounceMs = 50) {
    this.#send = send;
    this.#debounceMs = debounceMs;
  }

  /** Declare the whole desired set; the difference is what travels. */
  update(next: string[]): void {
    const change = delta(this.#current, next);
    this.#current = next;
    if (change.add.length === 0 && change.remove.length === 0) return;
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#send(change);
    }, this.#debounceMs);
  }

  /// Re-send everything, for a reconnection.
  ///
  /// Watches do not survive a dropped connection, and a client that resumed believing it was
  /// still being told about changes would show a tree that had quietly stopped updating —
  /// which is the failure FR-025 exists to prevent, arriving by a different route (FR-026b).
  reestablish(): void {
    if (this.#current.length > 0) this.#send({ add: [...this.#current], remove: [] });
  }

  /** What is currently asked for. */
  get current(): readonly string[] {
    return this.#current;
  }
}
