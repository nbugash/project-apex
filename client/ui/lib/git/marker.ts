/// One git state to what the tree shows for it.
///
/// **Pure, and the whole of the mapping.** Every presentational decision about a git state is
/// here rather than spread across the surfaces that render it, so a state added later is a
/// change in one file and a compile error in the others.
///
/// Tokens are named, never valued. `lint:ds` refuses a colour literal in markup, and naming the
/// token here means the design system can move the colour without this file knowing — which is
/// what makes it a design system rather than a palette somebody copied.

/** The five states §4.8 carries, lowercased at the Tauri boundary. */
export type GitState = 'modified' | 'untracked' | 'staged' | 'deleted' | 'conflict';

export interface GitMarker {
  /** One character, in the tree's mono font. */
  glyph: string;
  /** A design-system colour token name, to be used through `var()`. */
  token: string;
  /** For assistive technology, which gets the word rather than the letter. */
  label: string;
}

/// Glyph **and** luminance, not either alone.
///
/// The glyphs are distinct letters, so the states are told apart by shape with no colour at
/// all. The tokens are then chosen for separated luminance so that a reader who is scanning
/// colour rather than reading letters is not relying on hue either (FR-014, FR-015, SC-008).
/// Two independent channels, because a marker ten pixels tall is read as a smudge of colour as
/// often as it is read as a letter.
const MARKERS: Record<GitState, GitMarker> = {
  // Brightest: the only state that means work will be lost if it is ignored.
  conflict: { glyph: '!', token: '--color-neutral-100', label: 'conflicted' },
  untracked: { glyph: 'U', token: '--color-neutral-300', label: 'untracked' },
  modified: { glyph: 'M', token: '--color-accent-400', label: 'modified' },
  staged: { glyph: 'S', token: '--color-accent-2-500', label: 'staged' },
  // Dimmest: a file that is no longer there is the least actionable of the five.
  deleted: { glyph: 'D', token: '--color-neutral-600', label: 'deleted' },
};

/** The marker for a state, or `null` for a file with no git state at all (FR-016). */
export function markerFor(state: string | undefined | null): GitMarker | null {
  if (!state) return null;
  // A state this build does not know is shown as nothing, never as a default. Marking a file
  // with a state git never reported is worse than not marking it.
  return MARKERS[state as GitState] ?? null;
}

/** Every state, for tests and for any surface that needs a legend. */
export function allMarkers(): ReadonlyArray<readonly [GitState, GitMarker]> {
  return Object.entries(MARKERS) as ReadonlyArray<readonly [GitState, GitMarker]>;
}
