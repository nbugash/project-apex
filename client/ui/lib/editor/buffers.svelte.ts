/**
 * The open files, and everything true about them.
 *
 * **Module-level, not component state.** `EditorPanel` is unmounted whenever its tab is not the
 * focused one, and a model held in the component would lose the buffer, its base hash and its
 * dirty flag on every tab switch. The terminal had exactly this defect — `detach()` disposed the
 * instance, and nobody found it until the dock grew tabs to hide a panel behind. Designed out
 * here rather than found later.
 *
 * Rules live in `data-model.md`; this file implements them and says which is which.
 */

import { LoadedRegions } from './ranges';

/// How a save attempt ended. Four variants, never collapsed: a conflict means a colleague edited
/// the file and an unreachable engine means the link dropped, and what a developer does next
/// differs completely (FR-012).
export type WriteOutcome =
  | { kind: 'written'; sha256: string }
  | { kind: 'conflict' }
  | { kind: 'refused'; code: number; message: string }
  | { kind: 'unreachable' };

export class Buffer {
  readonly path: string;
  text = $state('');
  /// The hash the content had when read, or the hash a successful write returned. Nothing else
  /// sets it — see research.md, *Where the base hash lives*.
  base = $state<string | null>(null);
  dirty = $state(false);
  loaded = $state<LoadedRegions>(new LoadedRegions(0));
  ending = $state<WriteOutcome | null>(null);
  /// True while a write is in flight. FR-014: two saves of one file must not race into a wrong
  /// base.
  saving = $state(false);

  constructor(path: string) {
    this.path = path;
  }

  /// Editable only when every byte is held. A whole-file write of a partial buffer would replace
  /// the unloaded regions with nothing, and §4.8 carries content rather than a patch, so there
  /// is no safe partial write to fall back on (research.md, *Ranges*).
  get editable(): boolean {
    return this.loaded.complete();
  }

  /// Seed the buffer from what was read. Not an edit: this is the file, so nothing is dirty.
  fill(text: string, sha256: string, total: number, range?: [number, number]): void {
    this.text = text;
    this.base = sha256;
    const regions = new LoadedRegions(total);
    regions.add(range?.[0] ?? 0, range?.[1] ?? total);
    this.loaded = regions;
    this.dirty = false;
    this.ending = null;
  }

  /// An edit from the editor. Refused while regions are missing, rather than accepted and
  /// silently discarded at save time.
  applyEdit(text: string): boolean {
    if (!this.editable) return false;
    this.text = text;
    this.dirty = true;
    return true;
  }

  /// A write landed. The returned hash describes what is on disk, so it becomes the new base.
  adopt(sha256: string): void {
    this.base = sha256;
    this.dirty = false;
    this.ending = { kind: 'written', sha256 };
  }

  /// A write did not land.
  ///
  /// **The buffer stays dirty and its text is untouched** (FR-013, FR-011). The failing case is
  /// an optimistic clear: marking the buffer saved when the request goes out tells the developer
  /// their work is on the host when it is in flight, or lost.
  failed(outcome: WriteOutcome): void {
    this.ending = outcome;
    this.dirty = true;
  }

  /// Take the host's content, discarding local changes. The one escape from a conflict this
  /// feature offers (FR-012a); merging is F012's.
  reload(text: string, sha256: string, total: number): void {
    this.fill(text, sha256, total);
  }
}

export class BufferSet {
  /// An array rather than a map, for the reason the terminal's panel set gives: the count is the
  /// number of files a developer has open, so a linear scan is cheaper than the reactivity
  /// wrapper a keyed collection would need.
  buffers = $state<Buffer[]>([]);

  /// One buffer per path, however many times it is opened (FR-023). Two buffers of one file
  /// would mean two bases, and a save through one would silently revert the other.
  open(path: string): Buffer {
    const existing = this.buffers.find((b) => b.path === path);
    if (existing) return existing;
    const created = new Buffer(path);
    this.buffers.push(created);
    return created;
  }

  get(path: string): Buffer | undefined {
    return this.buffers.find((b) => b.path === path);
  }

  has(path: string): boolean {
    return this.buffers.some((b) => b.path === path);
  }

  close(path: string): void {
    const at = this.buffers.findIndex((b) => b.path === path);
    if (at >= 0) this.buffers.splice(at, 1);
  }
}

export const buffers = new BufferSet();
