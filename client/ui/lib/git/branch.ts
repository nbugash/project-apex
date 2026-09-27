/// Where the repository is, as the status bar says it.
///
/// Pure, so the three cases are testable without a status bar — and the three cases are the
/// point. git writes `(detached)` where a branch name goes, so an optional string would put
/// that text on screen as though it were a branch a developer could push (FR-018, FR-019).

import type { GitBranch } from './status.svelte';

export interface BranchLabel {
  /// What the developer reads. Never empty: an empty label renders as a gap that looks like a
  /// rendering fault rather than like "there is no repository here".
  text: string;
  /// The icon, from the design system's set.
  icon: string;
  /// For assistive technology, which needs the distinction spelled out rather than implied by
  /// an icon.
  title: string;
}

/// The label, or `null` when there is nothing to say.
///
/// `null` rather than a placeholder. A workspace that is not a repository has no branch, and
/// inventing "no branch" or "—" would occupy the space with a statement about git for a
/// developer who is not using it (FR-019, SC-009).
export function branchLabel(branch: GitBranch | null | undefined): BranchLabel | null {
  if (!branch) return null;
  switch (branch.kind) {
    case 'branch':
      // An empty name cannot be rendered as a branch. It means the engine sent a case it
      // could not fill, and showing nothing is the honest reading of that.
      return branch.name
        ? { text: branch.name, icon: 'ph-git-branch', title: `On branch ${branch.name}` }
        : null;
    case 'detached':
      // **Shortened for display only.** The full commit is what the engine sent and what the
      // title carries; seven characters is what git itself shows and what a developer
      // recognises.
      return branch.commit
        ? {
            text: branch.commit.slice(0, 7),
            icon: 'ph-git-commit',
            title: `Detached at ${branch.commit}`,
          }
        : null;
    case 'none':
      return null;
    default:
      // A case this build has not heard of. Nothing, rather than a guess: a newer core is not
      // a reason to put an unrecognised word on the status bar.
      return null;
  }
}
