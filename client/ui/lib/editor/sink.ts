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
  /// Where `text` begins. Not always what was asked for: a range boundary can land inside a
  /// multi-byte character, and the core trims the answer to whole characters rather than
  /// substituting U+FFFD for the fragment — which the developer would then save over the file.
  offset: number;
}

/// Why a read could not be answered, as cases rather than a sentence.
export interface ReadRefusal {
  kind: 'notText' | 'tooLarge' | 'notFound' | 'offline' | 'other';
  /// Present for `tooLarge`. "Too large" without the size tells a developer nothing to act on.
  total?: number;
  message: string;
}

export class ReadFailed extends Error {
  readonly refusal: ReadRefusal;
  constructor(refusal: ReadRefusal) {
    super(refusal.message);
    this.refusal = refusal;
  }
}

/// Turn the core's tagged failure into the cases the surface branches on.
///
/// The core sends `{ kind, detail }`; anything else came from Tauri itself (an unknown command,
/// a panic) and is reported as `other` rather than guessed at.
export function refusalFrom(e: unknown): ReadRefusal {
  const tagged = e as { kind?: string; detail?: unknown } | null;
  const message = String(e);
  switch (tagged?.kind) {
    case 'not_text':
      return { kind: 'notText', message };
    case 'too_large':
      return { kind: 'tooLarge', total: Number(tagged.detail ?? 0), message };
    case 'not_found':
      return { kind: 'notFound', message };
    case 'offline':
      return { kind: 'offline', message };
    default:
      return { kind: 'other', message };
  }
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
///
/// Recorded **here**, at the one place a request actually leaves, rather than wherever a caller
/// believes it issued one. SC-001 and SC-002 are counts of what left the process; a counter any
/// further up would count intentions, and the claim those numbers make — that typing reaches no
/// network — is worth exactly as much as the thing doing the counting.
function recordForAutomation(method: string, path: string, bytes?: number): void {
  if (!(import.meta.env.DEV || navigator.webdriver === true)) return;
  const w = window as unknown as { __apexEditorSent?: Array<Record<string, unknown>> };
  w.__apexEditorSent = w.__apexEditorSent ?? [];
  w.__apexEditorSent.push({ method, path, at: Date.now(), bytes: bytes ?? 0 });
}

/// The size of a response, recorded after it arrives.
///
/// Separate from the request record because a request's size is not what §4.1 caps -- the frame
/// carrying the answer is, and a range that asked for a legal amount can still be answered with
/// more than fits if anything miscounts.
function recordResponse(path: string, text: string): void {
  if (!(import.meta.env.DEV || navigator.webdriver === true)) return;
  const w = window as unknown as { __apexEditorReceived?: Array<Record<string, unknown>> };
  w.__apexEditorReceived = w.__apexEditorReceived ?? [];
  w.__apexEditorReceived.push({ path, bytes: new TextEncoder().encode(text).length });
}

/// How many requests have left, for a test that asserts a number rather than reads a list.
export function requestsIssued(): number {
  const w = window as unknown as { __apexEditorSent?: unknown[] };
  return w.__apexEditorSent?.length ?? 0;
}

const overIpc: EditorSink = {
  async read(path, range) {
    recordForAutomation(range ? 'readRange' : 'read', path);
    try {
      const chunk = (range
        ? await invoke('file_read_range', {
            path,
            offset: range[0],
            len: range[1] - range[0],
          })
        : await invoke('file_read', { path })) as Chunk;
      recordResponse(path, chunk.text);
      return chunk;
    } catch (e) {
      throw new ReadFailed(refusalFrom(e));
    }
  },

  async write(path, text, base) {
    recordForAutomation('write', path);
    try {
      // All four outcomes arrive in the success channel, already tagged. This used to read the
      // failure's *message* for the words "conflict" and "not connected" — which never appear
      // in either: the core's conflict says "the file changed on the host since it was read".
      // Every conflict was therefore reported as a plain refusal, and the developer never got
      // the reload that FR-012a exists to offer them. The distinction is made once, where the
      // §4.4 code is still in hand, and carried rather than reconstructed.
      return (await invoke('file_write', { path, content: text, base })) as WriteOutcome;
    } catch {
      // Nothing the core produced: an unknown command, a panic, a host that went away. The
      // write did not land and nothing on the far side changed, which is what `unreachable`
      // means. Reported as such rather than as a refusal, because a refusal would tell the
      // developer to change something about a file that is perfectly fine.
      return { kind: 'unreachable' };
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
