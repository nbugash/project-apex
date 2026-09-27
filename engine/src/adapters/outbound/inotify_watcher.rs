//! The one file in this repository that may name `inotify`.
//!
//! `engine/tests/inotify_confinement.rs` fails the build if the name appears anywhere else,
//! which is Principle VIII enforced rather than observed: everything that decides anything --
//! which paths are watched, when events collapse, when a flood becomes an invalidation -- sits
//! above the port and is tested against an in-memory double with no filesystem at all.
//!
//! Linux only. The client never watches in remote mode (§10.3) and local mode does not watch at
//! all in v1 (A-WATCHLOCAL), so there is no second backend to keep in step.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

/// Linux errno values, named rather than written inline at the match arm. `libc` is not a
/// dependency of this crate and adding one for two integers would be the wrong trade.
const ENOSPC: i32 = 28;
const ENOTDIR: i32 = 20;

use crate::application::ports::clock::Millis;
use crate::application::ports::file_watcher::{FileWatcher, WatchError};
use crate::domain::path::{CanonicalRoot, ResolvedPath};
use crate::domain::watch::{RawEvent, RawKind, WatchId};

/// What we ask the kernel for. Deliberately not `IN_ACCESS` or `IN_OPEN`: reading a file is not
/// a change, and a watcher that reported reads would flood on its own indexing.
fn mask() -> WatchMask {
    WatchMask::CREATE
        | WatchMask::MODIFY
        | WatchMask::DELETE
        | WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::DELETE_SELF
        | WatchMask::MOVE_SELF
        | WatchMask::CLOSE_WRITE
}

pub struct InotifyWatcher {
    inner: Inotify,
    root: PathBuf,
    /// Descriptor to the directory it observes, so an event's `name` can be made relative.
    directories: HashMap<WatchDescriptor, PathBuf>,
    ids: HashMap<u64, WatchDescriptor>,
    next: u64,
    buffer: Vec<u8>,
}

impl InotifyWatcher {
    pub fn new(root: &CanonicalRoot) -> std::io::Result<Self> {
        Ok(Self {
            inner: Inotify::init()?,
            root: root.as_path().to_path_buf(),
            directories: HashMap::new(),
            ids: HashMap::new(),
            next: 0,
            // 16 KiB holds roughly five hundred events. The kernel drops beyond its own queue
            // regardless and says so with an overflow, which is handled rather than hidden.
            buffer: vec![0; 16 * 1024],
        })
    }

    fn relative(&self, directory: &std::path::Path, name: Option<&std::ffi::OsStr>) -> String {
        let full = match name {
            Some(n) => directory.join(n),
            None => directory.to_path_buf(),
        };
        full.strip_prefix(&self.root)
            .unwrap_or(&full)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

impl FileWatcher for InotifyWatcher {
    fn watch(&mut self, directory: &ResolvedPath) -> Result<WatchId, WatchError> {
        let path = directory.as_path().to_path_buf();
        let descriptor = self.inner.watches().add(&path, mask()).map_err(|e| {
            match e.raw_os_error() {
                // The per-user watch ceiling. A resource limit, not a full disk, and the one
                // the workspace must stay open and browsable through (FR-005a).
                Some(ENOSPC) => WatchError::CapacityExhausted,
                Some(ENOTDIR) => WatchError::NotADirectory,
                _ => WatchError::Gone,
            }
        })?;
        self.next += 1;
        self.directories.insert(descriptor.clone(), path);
        self.ids.insert(self.next, descriptor);
        Ok(WatchId(self.next))
    }

    fn unwatch(&mut self, id: WatchId) -> Result<(), WatchError> {
        if let Some(descriptor) = self.ids.remove(&id.0) {
            self.directories.remove(&descriptor);
            // Already gone is success. The kernel drops a watch when its directory is removed,
            // so a client releasing afterwards must not be told it did something wrong.
            let _ = self.inner.watches().remove(descriptor);
        }
        Ok(())
    }

    fn poll(&mut self, timeout: Millis) -> Vec<RawEvent> {
        let deadline = Instant::now() + Duration::from_millis(timeout);
        loop {
            let mut buffer = std::mem::take(&mut self.buffer);
            let out = match self.inner.read_events(&mut buffer) {
                Ok(events) => {
                    let mut out = Vec::new();
                    for event in events {
                        let Some(directory) = self.directories.get(&event.wd).cloned() else {
                            continue;
                        };
                        let relative = self.relative(&directory, event.name);
                        let is_directory = event.mask.contains(EventMask::ISDIR);
                        let (size, modified) = stat(&self.root, &relative);
                        let kind = if event.mask.contains(EventMask::Q_OVERFLOW) {
                            RawKind::Overflow
                        } else if event.mask.contains(EventMask::CREATE) {
                            RawKind::Created
                        } else if event.mask.contains(EventMask::MOVED_FROM) {
                            RawKind::MovedFrom(event.cookie)
                        } else if event.mask.contains(EventMask::MOVED_TO) {
                            RawKind::MovedTo(event.cookie)
                        } else if event.mask.contains(EventMask::DELETE)
                            || event.mask.contains(EventMask::DELETE_SELF)
                        {
                            RawKind::Deleted
                        } else {
                            RawKind::Modified
                        };
                        out.push(RawEvent {
                            kind,
                            relative_path: relative,
                            is_directory,
                            size,
                            modified,
                        });
                    }
                    out
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Vec::new(),
                Err(_) => Vec::new(),
            };
            self.buffer = buffer;
            if !out.is_empty() {
                return out;
            }
            let now = Instant::now();
            if now >= deadline {
                return Vec::new();
            }
            // Short enough that the coalescer's deadline is honoured to within a few
            // milliseconds, long enough not to spin. The port's contract is only that `poll`
            // never blocks *longer* than the timeout.
            std::thread::sleep(Duration::from_millis(2).min(deadline - now));
        }
    }

    fn held(&self) -> usize {
        self.directories.len()
    }
}

/// Entry metadata (FR-013a). Absent for something already deleted, which is correct: a deleted
/// path has nothing to measure and the event carries none.
fn stat(root: &std::path::Path, relative: &str) -> (u64, i64) {
    let Ok(meta) = std::fs::metadata(root.join(relative)) else {
        return (0, 0);
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let _ = SystemTime::now();
    (meta.len(), modified)
}

/// How the composition root obtains a watcher without naming this library.
///
/// The confinement guard permits `inotify` in this file only, and a composition root that
/// constructed `InotifyWatcher` directly would be a second. Putting the platform decision in
/// the platform adapter keeps the rule at exactly one file, which is the version of the rule
/// worth having: any weakening is a judgement call, and a rule with one judgement call in it
/// soon has two.
pub fn factory() -> crate::adapters::outbound::watchers::WatcherFactory {
    Box::new(|root| {
        let watcher = InotifyWatcher::new(root).ok()?;
        let clock: std::sync::Arc<dyn crate::application::ports::clock::Clock> =
            std::sync::Arc::new(crate::adapters::outbound::system_clock::SystemClock);
        Some((Box::new(watcher) as Box<dyn FileWatcher>, clock))
    })
}

// ---- The git directory watch (F011, A-GITWATCH) ----

/// Two watches inside a repository's git directory, and nothing else.
///
/// A **second type with its own inotify instance**, not a mode of the watcher above. A-GITWATCH
/// requires that these events never become `workspace/onFileEvent`, and the way to guarantee
/// that is for there to be no code path between them: this type knows nothing about the
/// exclusion set, emits no `RawEvent`, and is reached through a different port.
///
/// It lives in this file because this is the one file permitted to name the library
/// (`inotify_confinement.rs`), not because it belongs to the watcher above.
pub struct InotifyGitWatch;

/// The directory is watched, and events filtered by name — **not** the two files directly.
///
/// git replaces `index` by writing `index.lock` and renaming it over the target. A watch on the
/// file follows the old inode, which after the first rename is an unlinked file nothing will
/// ever touch again: the watch would fire exactly once and then go quiet forever, on a
/// repository that looks perfectly healthy.
fn is_interesting(name: &std::ffi::OsStr) -> bool {
    matches!(name.to_str(), Some("HEAD") | Some("index"))
}

struct GitWatchThread {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl crate::application::ports::git_watch::GitWatchHandle for GitWatchThread {}

impl Drop for GitWatchThread {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl crate::application::ports::git_watch::GitWatch for InotifyGitWatch {
    fn watch(
        &self,
        git_dir: &std::path::Path,
        on_change: Box<dyn Fn() + Send + Sync>,
    ) -> Result<
        Box<dyn crate::application::ports::git_watch::GitWatchHandle>,
        crate::application::ports::git::GitFailure,
    > {
        use crate::application::ports::git::GitFailure;

        let inner = Inotify::init()
            .map_err(|e| GitFailure::Failed(format!("git watch could not start: {e}")))?;
        inner
            .watches()
            .add(
                git_dir,
                WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE,
            )
            .map_err(|e| GitFailure::Failed(format!("git watch could not attach: {e}")))?;

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("apex-git-watch".into())
            .spawn(move || {
                let mut inner = inner;
                let mut buffer = [0u8; 4096];
                while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                    // A blocking read would never notice the stop flag. A short timeout costs a
                    // wakeup per interval on an idle repository and makes shutdown prompt.
                    match inner.read_events(&mut buffer) {
                        Ok(events) => {
                            let mut fire = false;
                            for e in events {
                                if e.name.is_some_and(is_interesting) {
                                    fire = true;
                                }
                            }
                            if fire {
                                on_change();
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(25));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| GitFailure::Failed(format!("git watch thread: {e}")))?;

        Ok(Box::new(GitWatchThread {
            stop,
            handle: Some(handle),
        }))
    }
}

/// How the composition root obtains the git watch without naming this library.
///
/// The same reason `factory` above exists, and found the same way: the composition root named
/// `InotifyGitWatch` directly, and `inotify_confinement.rs` failed. A guard that catches the
/// second file on the day it appears is worth more than one that is argued with afterwards.
pub fn git_watch() -> std::sync::Arc<dyn crate::application::ports::git_watch::GitWatch> {
    std::sync::Arc::new(InotifyGitWatch)
}
