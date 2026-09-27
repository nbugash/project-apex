//! Outbound port: notice that a repository's git state may have changed.
//!
//! Separate from `FileWatcher`, and that separation is the requirement rather than a tidiness
//! preference. A-GITWATCH allows exactly two paths inside the git directory to be watched, and
//! requires their events never to become `workspace/onFileEvent`. Two ports with two
//! implementations means there is **no code path** between them — the guarantee holds because
//! nothing connects them, not because a filter stays correct while both features change.
//!
//! The port says only *that* something changed, never what. Which file moved inside the git
//! directory is git's business; the engine's response to either is the same — ask git again.

use crate::application::ports::git::GitFailure;
use std::path::Path;

/// Dropping this stops the watch.
pub trait GitWatchHandle: Send + Sync {}

pub trait GitWatch: Send + Sync {
    /// Watch `git_dir` for the two files that mean status may have changed.
    ///
    /// `on_change` is called on the watcher's own thread and must not block: it exists to wake
    /// the coalescer, which decides whether and when to actually run git.
    fn watch(
        &self,
        git_dir: &Path,
        on_change: Box<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn GitWatchHandle>, GitFailure>;
}
