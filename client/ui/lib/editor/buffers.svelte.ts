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
import { editorSink, ReadFailed, refusalFrom, type ReadRefusal } from './sink';

/// The largest file opened as text (plan.md, *Fixed Quantities*).
///
/// Duplicated from `MAX_TEXT_FILE` in `tauri_commands.rs` because the two runtimes cannot share
/// a constant. `editor-limits.test.ts` reads the Rust source and asserts they agree, so drift
/// fails a test rather than waiting to be noticed in review (Principle II).
export const MAX_TEXT_FILE = 64 * 1024 * 1024;

/// Above this a file is opened a window at a time (plan.md, *Fixed Quantities*). The same number
/// as `wire::MAX_INLINE_READ`, and checked against it by the same test.
export const CHUNK_THRESHOLD = 512 * 1024;

/// One window (plan.md, *Fixed Quantities*).
export const SCROLL_RANGE = 256 * 1024;

/// How long typing must stop before autosave writes (plan.md, *Fixed Quantities*, FR-007c).
///
/// Not §1.5's 50-100 ms: nobody waits for a save. It is bound by how long a developer tolerates
/// their work being unwritten, not by perceived latency -- short enough that a crash costs a
/// sentence, long enough that ordinary typing produces one write per pause rather than per word.
export const AUTOSAVE_DEBOUNCE_MS = 2000;

/// The length of `text` in **bytes**, which is what a range is expressed in.
///
/// Not `text.length`: that counts UTF-16 code units, so one emoji counts as two and every
/// region recorded after it is wrong by a byte or more. The error compounds along a file and
/// shows up as a range request for content that was already loaded, or a gap that never fills.
function byteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

/// How a save attempt ended. Four variants, never collapsed: a conflict means a colleague edited
/// the file and an unreachable engine means the link dropped, and what a developer does next
/// differs completely (FR-012).
export type WriteOutcome =
  | { kind: 'written'; sha256: string }
  | { kind: 'conflict' }
  | { kind: 'refused'; message: string }
  | { kind: 'unreachable' };

/// Why a file is not shown, or what has happened to it since it was.
///
/// Held separately from `ending`, which is about saving. A developer can need both at once — a
/// save that was refused *and* a file that has since vanished — and one field would let each
/// erase the other, leaving whichever arrived last as the whole story.
export type BufferNotice =
  | { kind: 'binary' }
  | { kind: 'tooLarge'; total: number; limit: number }
  /// Deleted on the host. Reported, never acted on: discarding the buffer would destroy unsaved
  /// work because somebody else removed the file (FR-025).
  | { kind: 'missing' }
  /// Changed on the host while this buffer had unsaved edits (FR-024b).
  | { kind: 'diverged' }
  | { kind: 'unavailable'; message: string };

export class Buffer {
  readonly path: string;
  text = $state('');
  /// The hash the content had when read, or the hash a successful write returned. Nothing else
  /// sets it — see research.md, *Where the base hash lives*.
  base = $state<string | null>(null);
  dirty = $state(false);
  loaded = $state<LoadedRegions>(new LoadedRegions(0));
  ending = $state<WriteOutcome | null>(null);
  /// What is wrong with the file itself, as opposed to with the last save.
  notice = $state<BufferNotice | null>(null);
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
    this.notice = null;
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
  ///
  /// Returns an already-open buffer **without re-reading it**. A reopen that refetched would
  /// discard unsaved work every time a developer clicked a tab they already had open.
  async open(path: string): Promise<Buffer> {
    const existing = this.buffers.find((b) => b.path === path);
    if (existing) return existing;
    const created = new Buffer(path);
    this.buffers.push(created);
    await load(created);
    return created;
  }

  /// The buffer for a path, created empty if absent and never read from the host.
  ///
  /// For a restored tab whose content is fetched when it is focused (FR-021): the tab exists
  /// before its content does, and a component rendering it needs something to bind to.
  ensure(path: string): Buffer {
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

/// Read a file into a buffer, deciding whole against windowed by what the host says.
///
/// Whole is attempted **first**, not decided from a size fetched beforehand. A size-first design
/// costs a round trip on every open to save one on the rare large file, and SC-002 counts the
/// common case (FR-018).
export async function load(b: Buffer): Promise<void> {
  try {
    const chunk = await editorSink().read(b.path, null);
    b.fill(chunk.text, chunk.sha256, chunk.total, [
      chunk.offset,
      chunk.offset + byteLength(chunk.text),
    ]);
  } catch (e) {
    const refusal: ReadRefusal = e instanceof ReadFailed ? e.refusal : refusalFrom(e);
    if (refusal.kind === 'notText') {
      // Declined rather than shown as mojibake. F017 is the surface that will render it; naming
      // it makes the refusal a schedule rather than a dead end (FR-006).
      b.notice = { kind: 'binary' };
      return;
    }
    if (refusal.kind === 'tooLarge') {
      const total = refusal.total ?? 0;
      if (total > MAX_TEXT_FILE) {
        // A window that stops responding is worse than a refusal that says how big the file is
        // and what the limit was.
        b.notice = { kind: 'tooLarge', total, limit: MAX_TEXT_FILE };
        return;
      }
      await loadWindow(b, total, 0);
      return;
    }
    if (refusal.kind === 'notFound') {
      // A restored tab whose file has since gone. Reported as missing rather than presented as
      // an empty document, which is what FR-022 forbids: an empty editor looks like a file that
      // is there and has nothing in it, and saving it would create one.
      b.notice = { kind: 'missing' };
      return;
    }
    b.notice = { kind: 'unavailable', message: refusal.message };
  }
}

/// Read one window of a large file (FR-017).
export async function loadWindow(b: Buffer, total: number, offset: number): Promise<void> {
  try {
    const end = Math.min(offset + SCROLL_RANGE, total);
    const chunk = await editorSink().read(b.path, [offset, end]);
    const from = chunk.offset;
    const to = from + byteLength(chunk.text);
    if (b.base === null) {
      b.fill(chunk.text, chunk.sha256, total, [from, to]);
      return;
    }
    // A later window of a file that moved underneath the read is not part of the same file. The
    // digest describes the whole file, so a change in it means every window already held is
    // suspect, and stitching the new one on would produce a document that never existed
    // anywhere (FR-021, `FileChunk::sha256`).
    if (chunk.sha256 !== b.base) {
      b.notice = { kind: 'diverged' };
      return;
    }
    b.text += chunk.text;
    b.loaded.add(from, to);
  } catch (e) {
    const refusal: ReadRefusal = e instanceof ReadFailed ? e.refusal : refusalFrom(e);
    b.notice = { kind: 'unavailable', message: refusal.message };
  }
}

/// Load whatever is still missing, so a partial buffer can be edited (T052, research.md).
export async function loadRest(b: Buffer): Promise<void> {
  // Bounded by the region count rather than by a `while (!complete)`: a host that keeps
  // answering with less than was asked for would spin forever, and an editor that hangs on
  // "make this editable" is worse than one that says it could not.
  const total = b.loaded.total;
  for (let guard = 0; guard < Math.ceil(total / SCROLL_RANGE) + 1; guard += 1) {
    if (b.editable) return;
    const missing = b.loaded.missingFor(0, total);
    if (missing.length === 0) return;
    const before = byteLength(b.text);
    await loadWindow(b, total, missing[0]![0]);
    if (b.notice !== null) return;
    if (byteLength(b.text) === before) return;
  }
}

/// Save, and refuse to start a second write while one is outstanding.
///
/// FR-014. Two saves of one file in flight together would each carry the base the buffer held
/// when it started; whichever landed second would be refused for a conflict the developer
/// caused by saving twice, or -- worse, had the bases been adopted out of order -- would write
/// over the first with a base the host had already moved past.
export async function save(b: Buffer): Promise<WriteOutcome | null> {
  if (b.saving || !b.dirty || b.base === null) return null;
  // A whole-file write of a partial buffer replaces the unloaded regions with nothing, and §4.8
  // carries content rather than a patch, so there is no safe partial write to fall back on.
  if (!b.editable) return null;
  b.saving = true;
  try {
    const outcome = await editorSink().write(b.path, b.text, b.base);
    if (outcome.kind === 'written') b.adopt(outcome.sha256);
    else b.failed(outcome);
    return outcome;
  } finally {
    b.saving = false;
  }
}

/// What a file event means for an open buffer (A-WRITEECHO, FR-024, FR-024a, FR-025).
///
/// The event alone says nothing. A save of this file produces one, and reporting "changed on the
/// host" for the developer's own write is the noise that makes every later notice ignorable.
/// The hash decides: equal to the base means we are hearing ourselves.
export async function onFileEvent(b: Buffer, event: 'changed' | 'deleted'): Promise<void> {
  if (event === 'deleted') {
    // Reported, never acted on. Discarding the buffer would destroy unsaved work because
    // somebody else removed the file, and the developer may well want to save it back (FR-025).
    b.notice = { kind: 'missing' };
    return;
  }

  const current = await editorSink().hash(b.path);
  if (current !== null && current === b.base) return;

  if (b.dirty) {
    // Told, not overwritten. Replacing a dirty buffer discards work the developer has not saved
    // and cannot recover, which is the one outcome this feature exists to prevent (FR-024b).
    b.notice = { kind: 'diverged' };
    return;
  }

  // Clean, so there is nothing to lose by taking the host's version -- including when the hash
  // could not be obtained, where refetching is the cheap safe answer and guessing is not.
  await load(b);
}
