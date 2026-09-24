//! Turning "what the client cares about" into "what the host observes".
//!
//! The client sends folder paths for expanded folders and **file** paths for open editors. It
//! sends reasons, not conclusions, because a folder holding an open file is one path for two
//! reasons: if the client resolved that itself, unwatching on a collapse could not be told from
//! unwatching on a tab close, and collapsing a folder would silently stop reporting a file still
//! open inside it (FR-003c, FR-004, A-WATCHSCOPE).

use apex_protocol::wire::{RefusalReason, WatchRefusal, WatchResult};

use crate::application::exclusions::ExclusionSet;
use crate::application::ports::file_system::FileSystem;
use crate::application::ports::file_watcher::{FileWatcher, WatchError};
use crate::domain::path::{CanonicalRoot, PathRefusal, ResolvedPath};
use crate::domain::watch::WatchSet;

/// Directories the host must observe for one requested path.
///
/// A file's parent, or the folder itself, plus every ancestor up to the root. Ancestors are
/// watched because a rename of an ancestor changes every path beneath it and no descendant
/// watch would see it.
fn directories_for(root: &CanonicalRoot, target: &ResolvedPath, is_directory: bool) -> Vec<String> {
    let root_str = root.as_path().to_string_lossy().into_owned();
    let mut current = target.as_path().to_path_buf();
    if !is_directory {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        }
    }
    let mut out = Vec::new();
    loop {
        let shown = current.to_string_lossy().into_owned();
        out.push(shown.clone());
        if shown == root_str {
            break;
        }
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => break,
        }
    }
    out
}

/// Add every requested path to the watched set, and report what could not be watched.
///
/// Refusals are returned, never raised. Exhausted capacity is the expected case on a large
/// repository and the workspace must stay open and browsable through it (FR-005a); an error
/// would take the whole call down for one path.
#[allow(clippy::too_many_arguments)]
pub fn watch_paths(
    root: &CanonicalRoot,
    set: &mut WatchSet,
    watcher: &mut dyn FileWatcher,
    exclusions: &ExclusionSet,
    fs: &dyn FileSystem,
    requested: &[String],
) -> WatchResult {
    let mut refused = Vec::new();
    let mut accepted: usize = 0;

    for path in requested {
        let resolved = match ResolvedPath::resolve(root, path, fs) {
            Ok(r) => r,
            Err(PathRefusal::NotFound) => {
                // Not an error for the whole call. A folder deleted on the host while the
                // client was disconnected would otherwise fail the entire re-establishment and
                // leave everything unwatched -- the silence FR-026b exists to close.
                refused.push(WatchRefusal {
                    path: path.clone(),
                    reason: RefusalReason::NotFound,
                });
                continue;
            }
            Err(PathRefusal::Refused) => {
                // §4.7 is normative for every method, so a path escaping the root fails the
                // call rather than degrading to a per-path note. Deliberately inconsistent
                // with every other refusal here: capacity and a missing folder are conditions,
                // an escape is an attack surface.
                return WatchResult {
                    watching: set.len() as u32,
                    refused: Vec::new(),
                };
            }
        };

        if exclusions.is_excluded(path, true) {
            // Reported rather than silently skipped: silence is indistinguishable from a
            // working watch, which is the one outcome FR-005 forbids.
            refused.push(WatchRefusal {
                path: path.clone(),
                reason: RefusalReason::Excluded,
            });
            continue;
        }

        let is_directory = fs
            .metadata(resolved.as_path())
            .map(|m| m.is_directory)
            .unwrap_or(false);
        let mut all_watched = true;
        for directory in directories_for(root, &resolved, is_directory) {
            let target = ResolvedPath::resolve_absolute_for_watch(root, &directory);
            let (_, fresh) = set.acquire(target.clone());
            if !fresh {
                continue;
            }
            if let Err(why) = watcher.watch(&target) {
                set.release(&target);
                all_watched = false;
                refused.push(WatchRefusal {
                    path: path.clone(),
                    reason: match why {
                        WatchError::CapacityExhausted => RefusalReason::Capacity,
                        WatchError::NotADirectory => RefusalReason::NotADirectory,
                        WatchError::Gone => RefusalReason::NotFound,
                    },
                });
                break;
            }
        }
        if all_watched {
            accepted += 1;
        }
    }

    let _ = accepted;
    WatchResult {
        watching: set.len() as u32,
        refused,
    }
}

/// Drop one reason for each requested path, releasing host watches that nothing wants.
pub fn unwatch_paths(
    root: &CanonicalRoot,
    set: &mut WatchSet,
    watcher: &mut dyn FileWatcher,
    fs: &dyn FileSystem,
    requested: &[String],
) -> u32 {
    for path in requested {
        let Ok(resolved) = ResolvedPath::resolve(root, path, fs) else {
            continue;
        };
        let is_directory = fs
            .metadata(resolved.as_path())
            .map(|m| m.is_directory)
            .unwrap_or(false);
        for directory in directories_for(root, &resolved, is_directory) {
            let target = ResolvedPath::resolve_absolute_for_watch(root, &directory);
            if let Some(id) = set.release(&target) {
                let _ = watcher.unwatch(id);
            }
        }
    }
    set.len() as u32
}

/// Release everything, for a workspace closing or a connection dropping (FR-004).
pub fn release_all(set: &mut WatchSet, watcher: &mut dyn FileWatcher) {
    for id in set.drain_all() {
        let _ = watcher.unwatch(id);
    }
}
