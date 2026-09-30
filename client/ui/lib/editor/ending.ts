/**
 * What a developer is told when a save ends.
 *
 * Pure, and separate from the component, so the wording and — far more importantly — the
 * *distinctions* are testable without a window. FR-012 is a requirement about what a person
 * understands, and the only way to test that honestly is to assert on the words.
 */

import type { WriteOutcome } from './buffers.svelte';

export interface OutcomeLabel {
  /// One line, in the developer's terms. Never a code, never an exception's text.
  title: string;
  /// What it means for their work, or null when nothing needs saying.
  detail: string | null;
  /// Whether to offer discarding local changes and taking the host's content (FR-012a).
  ///
  /// The **only** escape this feature offers. There is deliberately no "overwrite anyway":
  /// re-reading the host's hash and writing over it destroys a colleague's work silently, which
  /// §11 names as the failure this product cannot afford (FR-012b).
  offersReload: boolean;
  tone: 'ok' | 'warning' | 'error';
}

export function describeOutcome(o: WriteOutcome): OutcomeLabel {
  switch (o.kind) {
    case 'written':
      return { title: 'Saved', detail: null, offersReload: false, tone: 'ok' };

    case 'conflict':
      // Says who and says what survived. "Conflict" alone is a word about the system; a
      // developer needs to know somebody else edited the file and that their work is still here.
      return {
        title: 'Someone else changed this file on the host',
        detail:
          'Your changes are still here and have not been saved. Look at what changed before deciding.',
        offersReload: true,
        tone: 'warning',
      };

    case 'heldLocally':
      // A success, and said as one. The developer needs to know two things: the work is safe, and
      // the host does not have it yet -- so both are here, because "Saved" alone would be the
      // confusion §11.2 forbids and "Not saved" would be false.
      return {
        title: 'Held on this machine',
        detail:
          'Saved locally. It will be sent to the host when the connection returns, and you will be asked about anything that collided.',
        offersReload: false,
        tone: 'ok',
      };

    case 'unreachable':
      // About the link, and never about the file. Offering a reload here would discard the
      // developer's work to fetch a version that is not reachable either.
      //
      // **F012 left this alone, and a task said to change it.** T031b argued that an offline save
      // is now held, so this case must mean the write could not even be held locally. That is
      // wrong given how the write path routes: `file_write` sends a save to the retainer *before*
      // `EditFile` is reached whenever the connection state is anything but connected, and a retain
      // that fails returns `Refused` with its reason. So `unreachable` still means exactly what
      // F006 made it mean -- a request that went out believing the link was up and never landed --
      // and F006's own tests, which require this case to name the link and to promise the work is
      // still here, were right to fail the rewrite.
      return {
        title: 'Not saved: the engine could not be reached',
        detail: 'Your changes are still here. Saving again once the connection returns will work.',
        offersReload: false,
        tone: 'error',
      };

    case 'refused':
      // The engine considered it and said no, so retrying unchanged fails identically. The
      // engine's own words are carried rather than paraphrased: it knows why and this does not.
      return {
        title: 'Not saved: the host refused the write',
        detail: o.message,
        offersReload: false,
        tone: 'error',
      };
  }
}
