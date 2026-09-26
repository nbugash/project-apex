/**
 * Where the editor sends what a person does to a file.
 *
 * A seam rather than a direct `invoke`, for the reason `terminal/sink.ts` gives and one of its
 * own. The buffer's rules are testable without a Tauri host, which is most of what this feature
 * has to get right; and SC-001 counts the requests typing issues, which means something has to
 * be counting, and a component calling `invoke` directly counts nothing.
 */

import { invoke } from '@tauri-apps/api/core';
import type { WriteOutcome } from './buffers.svelte';

export interface Chunk {
  /// The bytes, as text. Non-UTF-8 content never reaches here — it is declined on the way in.
  text: string;
  /// The whole file's hash, not the chunk's: a base describes the file.
  sha256: string;
  /// The whole file's size, so a partial read knows what it is part of.
  total: number;
}

export interface EditorSink {
  /// `range` null reads the whole file.
  read(path: string, range: [number, number] | null): Promise<Chunk>;
  write(path: string, text: string, base: string): Promise<WriteOutcome>;
  /// The file's current hash, for deciding whether a file event means anything (A-WRITEECHO).
  hash(path: string): Promise<string | null>;
}

/// What the editor asked for, for the end-to-end suite to read back.
///
/// Recorded only under automation, on the same terms as the terminal's: a page that logged every
/// file a developer opened would be doing what this product exists to avoid.
function recordForAutomation(method: string, path: string): void {
  if (!(import.meta.env.DEV || navigator.webdriver === true)) return;
  const w = window as unknown as { __apexEditorSent?: Array<Record<string, string>> };
  w.__apexEditorSent = w.__apexEditorSent ?? [];
  w.__apexEditorSent.push({ method, path });
}

const overIpc: EditorSink = {
  async read(path, range) {
    recordForAutomation(range ? 'readRange' : 'read', path);
    return range
      ? ((await invoke('file_read_range', {
          path,
          offset: range[0],
          len: range[1] - range[0],
        })) as Chunk)
      : ((await invoke('file_read', { path })) as Chunk);
  },

  async write(path, text, base) {
    recordForAutomation('write', path);
    try {
      const sha256 = (await invoke('file_write', { path, content: text, base })) as string;
      return { kind: 'written', sha256 };
    } catch (e) {
      // The shape of the failure is the whole point. A conflict is somebody else's edit and an
      // unreachable engine is the link; collapsing them would make the developer's next action
      // a guess (FR-012).
      const message = String(e);
      if (message.includes('conflict')) return { kind: 'conflict' };
      if (message.includes('not connected')) return { kind: 'unreachable' };
      return { kind: 'refused', code: 0, message };
    }
  },

  async hash(path) {
    recordForAutomation('hash', path);
    try {
      return (await invoke('file_hash', { path })) as string;
    } catch {
      // A hash we cannot obtain is not evidence of anything, and the caller is deciding whether
      // a file changed. Answering null says "unknown" rather than inventing "changed".
      return null;
    }
  },
};

let active: EditorSink = overIpc;

export function editorSink(): EditorSink {
  return active;
}

/// Swap the sink. Returns the previous one, so a test puts it back rather than leaving the next
/// one talking to a recorder.
export function setEditorSink(sink: EditorSink): EditorSink {
  const previous = active;
  active = sink;
  return previous;
}
