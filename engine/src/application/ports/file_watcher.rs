//! Outbound port: observing directories.
//!
//! A capability, not `inotify`. The adapter behind it is the only file in the repository
//! permitted to name that library (`engine/tests/inotify_confinement.rs` fails the build
//! otherwise), and everything that decides anything -- which paths are watched, when events
//! collapse, when a flood becomes an invalidation -- sits above this line and is tested
//! against an in-memory double with no filesystem at all.

use crate::application::ports::clock::Millis;
use crate::domain::path::ResolvedPath;
use crate::domain::watch::{RawEvent, WatchId};

/// Why a directory could not be watched.
///
/// Values, not panics. Exhausted host capacity is the expected case on a large repository and
/// the workspace must stay open and browsable through it (FR-005a), so the port hands the
/// condition back and the use case turns it into a refusal the developer is told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchError {
    /// The host will not hold another watch. On Linux this is `ENOSPC` from the kernel's
    /// per-user limit, which is a resource ceiling rather than a disk being full.
    CapacityExhausted,
    NotADirectory,
    /// It was there when the path resolved and is not there now.
    Gone,
}

/// **`Send` but not `Sync`**, unlike the engine's other ports.
///
/// `FileSystem` and its kin are `Send + Sync` because any thread may ask them anything. This
/// one is owned by exactly one thread -- the watcher thread holds the descriptor, polls it and
/// feeds the coalescer -- and `Sync` would advertise a sharing that must not happen: two
/// threads draining one event queue lose events between them, which is the silence the watch
/// requirements exist to forbid. The narrower bound is the design, stated in the type.
pub trait FileWatcher: Send {
    /// Begin observing one directory's immediate children.
    fn watch(&mut self, directory: &ResolvedPath) -> Result<WatchId, WatchError>;

    /// Stop observing. Releasing something already released is not an error, which is what
    /// makes a client's re-sent set idempotent rather than a sequence to get exactly right.
    fn unwatch(&mut self, id: WatchId) -> Result<(), WatchError>;

    /// Whatever arrived within `timeout`, possibly nothing.
    ///
    /// Must not block longer than `timeout`: the coalescer computes when its next window
    /// closes and asks for exactly that long, so over-blocking delays every pending event and
    /// no test that asserts on content would notice.
    fn poll(&mut self, timeout: Millis) -> Vec<RawEvent>;

    /// How many host watches are held.
    ///
    /// The criteria about watch counts assert through this rather than reading `/proc`, which
    /// would make them Linux-only and root-dependent. Note it is **not** the size of the
    /// client's requested set: the two differ by ancestors and the workspace root.
    fn held(&self) -> usize;
}
