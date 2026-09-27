//! Outbound port: what the engine needs from git, and nothing more.
//!
//! A port rather than a direct call to a subprocess, for the reason Principle VIII gives and one
//! specific to this feature: the hard parts here are a **parser** and a **coalescer**, and both
//! are testable against captured output and a fake clock only if the thing that shells out sits
//! behind a seam. `GitCli` is the adapter; the use case never sees a `Command`.

use crate::domain::path::ResolvedPath;
use apex_protocol::wire::{BranchPosition, GitChange, GitDiffResult};
use std::path::PathBuf;

/// One complete answer about a workspace: where the repository is, and what differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub branch: BranchPosition,
    pub changes: Vec<GitChange>,
}

impl StatusSnapshot {
    /// What a workspace with no repository, or a host with no git, reports.
    ///
    /// A named constructor rather than `Default`, because "there is no repository here" is a
    /// deliberate answer this feature gives in two distinct situations, not the absence of one.
    pub fn nothing() -> Self {
        Self {
            branch: BranchPosition::None,
            changes: Vec::new(),
        }
    }
}

/// Why git could not answer.
///
/// `NotARepository` and `GitUnavailable` are kept apart **inside the engine** even though both
/// reach the client as the same empty status. The client's behaviour is identical, so collapsing
/// them on the wire costs nothing; the engine's diagnostics distinguish a workspace that simply
/// is not a repository — ordinary, common, not worth a word — from a host missing a tool, which
/// is a configuration somebody can fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitFailure {
    NotARepository,
    GitUnavailable,
    Failed(String),
}

impl GitFailure {
    /// Whether this is an ordinary absence rather than something to report.
    ///
    /// Both absences degrade to an empty status (FR-027, FR-028); only `Failed` says something
    /// went wrong that was expected to work.
    pub fn is_absence(&self) -> bool {
        matches!(self, Self::NotARepository | Self::GitUnavailable)
    }
}

pub trait Git: Send + Sync {
    /// Everything that differs, and where the repository is, in one answer.
    fn status(&self, root: &ResolvedPath) -> Result<StatusSnapshot, GitFailure>;

    /// Which lines of one file differ. Coordinates only — there is no parameter and no return
    /// path here through which content could travel (§12.3).
    fn file_diff(&self, root: &ResolvedPath, relative: &str) -> Result<GitDiffResult, GitFailure>;

    /// The directory holding `HEAD` and `index`, which is **not always** `<root>/.git`.
    ///
    /// In a linked worktree or a submodule that path is a file holding a `gitdir:` pointer, and
    /// the real directory lives elsewhere — verified in research.md against git 2.43. A caller
    /// that assumed the obvious path would watch nothing and never update, on a workspace that
    /// otherwise behaves perfectly.
    fn git_dir(&self, root: &ResolvedPath) -> Result<PathBuf, GitFailure>;
}
