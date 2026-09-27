//! Turning pages of git status into one replacement, or into nothing at all.
//!
//! **This is the file where the one interesting mistake lives.** A status too large for a frame
//! arrives as a notification carrying the first page and a cursor; the rest is pulled. An
//! implementation that applied the notification as it stood would mark every path beyond the
//! first page as unchanged — and would pass every test that looks at a single message, on every
//! repository small enough to fit in one (A-GITPAGE).
//!
//! So the rule here is all-or-nothing: accumulate until the page with no cursor arrives, then
//! replace in one transaction. Anything that goes wrong in between leaves what was already
//! shown exactly as it was, because a partly-correct picture of what has changed is worse than
//! a slightly old one — the developer cannot tell which parts are which.

use crate::application::ports::git_provider::GitProvider;
use crate::application::ports::workspace_cache::{GitProjection, WorkspaceCache};
use crate::domain::workspace::{RelPath, WorkspaceId};
use apex_protocol::wire::{GitChange, GitStatusUpdate};
use std::sync::Arc;

/// How many pages one pull may take before it is abandoned.
///
/// A guard against an engine that returns a cursor forever, which is a bug this client cannot
/// fix and must not hang on (Principle VI: our own engine is untrusted input too). At the
/// engine's cap of 1000 entries per page this admits a million changed paths, which is far
/// beyond any repository a person works in and far short of forever.
const MAX_PAGES: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discarded {
    /// A later page was refused, or the connection went. Nothing was written.
    PullFailed(String),
    /// More pages arrived than `MAX_PAGES` allows.
    TooManyPages,
    /// The replacement itself failed — including a workspace this client does not have open,
    /// which the store refuses because git rows belong to a registered workspace.
    CommitFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The whole snapshot arrived and replaced what was there.
    Applied {
        paths: usize,
        pages: usize,
        /// Entries this client refused because their path escapes the workspace (FR-011).
        /// Never hidden: a number a test can assert on, as `apply_file_event` does.
        refused: usize,
    },
    Discarded(Discarded),
}

pub struct ApplyGitStatus {
    git: Arc<dyn GitProvider>,
    cache: Arc<dyn WorkspaceCache>,
}

impl ApplyGitStatus {
    pub fn new(git: Arc<dyn GitProvider>, cache: Arc<dyn WorkspaceCache>) -> Self {
        Self { git, cache }
    }

    /// Apply one `git/onStatusUpdate`, pulling any pages it says remain.
    pub async fn apply(&self, update: GitStatusUpdate) -> ApplyOutcome {
        let workspace = WorkspaceId(update.workspace_id.0.clone());
        self.accumulate(
            &workspace,
            update.current_branch,
            update.changes,
            update.next_cursor,
        )
        .await
    }

    /// Which lines of one file differ.
    ///
    /// Not applied to anything: a diff belongs to the editor showing that file, and storing it
    /// would hold the whole repository's diffs for the sake of the one on screen.
    pub async fn file_diff(
        &self,
        workspace: &WorkspaceId,
        relative_path: &str,
    ) -> crate::application::ports::workspace_provider::ProviderResult<
        apex_protocol::wire::GitDiffResult,
    > {
        self.git.file_diff(workspace, relative_path).await
    }

    /// Ask for a workspace's status outright, and apply the answer.
    ///
    /// **Asking is also what subscribes.** The engine begins watching a repository when a
    /// client first asks about it, so without this call nothing is ever watched and no
    /// `git/onStatusUpdate` is ever sent -- which is a client that is correct in every part and
    /// shows nothing. Found by the end-to-end spec and by nothing else: every unit test here
    /// starts from an update that had already arrived.
    pub async fn refresh(&self, workspace: &WorkspaceId) -> ApplyOutcome {
        match self.git.status(workspace, None).await {
            Ok(first) => {
                self.accumulate(
                    workspace,
                    first.current_branch,
                    first.changes,
                    first.next_cursor,
                )
                .await
            }
            Err(e) => ApplyOutcome::Discarded(Discarded::PullFailed(format!("{e:?}"))),
        }
    }

    async fn accumulate(
        &self,
        workspace: &WorkspaceId,
        branch: apex_protocol::wire::BranchPosition,
        first: Vec<GitChange>,
        first_cursor: Option<String>,
    ) -> ApplyOutcome {
        let workspace = workspace.clone();
        let mut accumulated: Vec<GitChange> = Vec::new();
        let mut refused = 0usize;
        let mut pages = 1usize;

        keep(&mut accumulated, &mut refused, first);
        let mut cursor = first_cursor;

        while let Some(next) = cursor {
            if pages >= MAX_PAGES {
                return ApplyOutcome::Discarded(Discarded::TooManyPages);
            }
            match self.git.status(&workspace, Some(&next)).await {
                Ok(page) => {
                    pages += 1;
                    keep(&mut accumulated, &mut refused, page.changes);
                    cursor = page.next_cursor;
                }
                // **Not retried and not partially applied.** A refused cursor means the engine's
                // snapshot is gone; asking again without one would start a second snapshot and
                // splice it onto the first (contracts/git-status.md).
                Err(e) => return ApplyOutcome::Discarded(Discarded::PullFailed(format!("{e:?}"))),
            }
        }

        let paths = accumulated.len();
        let projection = GitProjection {
            branch,
            changes: accumulated,
        };
        // Only now, and in one transaction. Everything above is in memory precisely so that a
        // failure at any point costs the pull rather than the picture.
        match self.cache.replace_git_status(&workspace, &projection) {
            Ok(()) => ApplyOutcome::Applied {
                paths,
                pages,
                refused,
            },
            Err(e) => ApplyOutcome::Discarded(Discarded::CommitFailed(format!("{e}"))),
        }
    }
}

/// Keep the entries whose paths this client can vouch for, and count the rest.
///
/// The engine contains paths before emitting and the client contains them again on arrival. Not
/// redundant: this check protects against a bug in our own engine, which is the failure a
/// one-sided boundary cannot catch at all (Principle VI, contracts/git-status.md).
fn keep(into: &mut Vec<GitChange>, refused: &mut usize, changes: Vec<GitChange>) {
    for change in changes {
        match RelPath::parse(&change.path) {
            Ok(path) => into.push(GitChange {
                path: path.as_str().to_owned(),
                status: change.status,
            }),
            Err(_) => *refused += 1,
        }
    }
}
