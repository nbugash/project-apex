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
        let clock: Box<dyn crate::application::ports::clock::Clock> = Box::new(SystemClock);
        Some((Box::new(watcher) as Box<dyn FileWatcher>, clock))
    })
}

/// The real clock. Beside the only adapter that needs one.
struct SystemClock;

impl crate::application::ports::clock::Clock for SystemClock {
    fn now(&self) -> Millis {
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as Millis)
            .unwrap_or(0)
    }
}
