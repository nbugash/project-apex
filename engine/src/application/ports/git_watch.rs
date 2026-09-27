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

/// Tell the git subsystem that a workspace may have changed, carrying **no detail at all**.
///
/// The second of A-GITNUDGE's two triggers. A-GITWATCH's two watches see every index-only
/// change and no ordinary save, because saving a tracked file writes neither `HEAD` nor
/// `index`; the workspace's own file events see the opposite. Both wake one coalescer, so a
/// change that raises both still costs one git run.
///
/// **A workspace and nothing else.** The direction added here is workspace-to-git, and the port
/// is shaped so nothing can travel the other way: there is no event type in the signature, so
/// there is nothing for a git event to be turned into. That is what keeps A-GITWATCH's
/// separation true by construction rather than by care.
pub trait StatusNudge: Send + Sync {
    fn nudge(&self, workspace: &apex_protocol::wire::WorkspaceId);
}
