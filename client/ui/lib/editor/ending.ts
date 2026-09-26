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

    case 'unreachable':
      // About the link, and never about the file. Offering a reload here would discard the
      // developer's work to fetch a version that is not reachable either.
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
