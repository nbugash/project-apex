//! What the engine observes, and what it decides to say about it.
//!
//! Nothing here touches a filesystem or a clock. The raw events arrive from a port and the
//! time arrives as a number, which is what makes the volume requirements -- a thousand writes
//! in a second, ten thousand changes at once -- arithmetic rather than a test that sleeps.

use std::collections::BTreeMap;

use crate::domain::path::ResolvedPath;

/// A handle to one host watch. Opaque: only the adapter that minted it knows what it indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WatchId(pub u64);

/// One watched directory, and why it is watched.
///
/// The reasons are a count rather than a set because two folders can want the same ancestor and
/// the only question ever asked is whether anything still wants it. A watch released while
/// something still depends on it is the failure FR-004 and FR-003c describe from two directions.
#[derive(Debug, Clone)]
pub struct Watch {
    pub id: WatchId,
    pub directory: ResolvedPath,
    reasons: usize,
}

impl Watch {
    pub fn reasons(&self) -> usize {
        self.reasons
    }
}

/// The directories watched for one workspace.
///
/// Idempotent in both directions: adding a directory twice yields one watch with two reasons,
/// and removing it twice is not an error. That is what lets a reconnecting client re-establish
/// everything with a single call instead of replaying a history it might mis-remember.
#[derive(Debug, Default)]
pub struct WatchSet {
    watches: BTreeMap<String, Watch>,
    next: u64,
}

impl WatchSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the watch, and whether the host needs to be asked for a new one.
    pub fn acquire(&mut self, directory: ResolvedPath) -> (WatchId, bool) {
        let key = directory.as_path().to_string_lossy().into_owned();
        if let Some(existing) = self.watches.get_mut(&key) {
            existing.reasons += 1;
            return (existing.id, false);
        }
        self.next += 1;
        let id = WatchId(self.next);
        self.watches.insert(
            key,
            Watch {
                id,
                directory,
                reasons: 1,
            },
        );
        (id, true)
    }

    /// Drop one reason. Returns the id only when the last reason went, meaning the host watch
    /// should now be released.
    pub fn release(&mut self, directory: &ResolvedPath) -> Option<WatchId> {
        let key = directory.as_path().to_string_lossy().into_owned();
        let entry = self.watches.get_mut(&key)?;
        entry.reasons -= 1;
        if entry.reasons == 0 {
            let id = entry.id;
            self.watches.remove(&key);
            return Some(id);
        }
        None
    }

    pub fn len(&self) -> usize {
        self.watches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    pub fn contains(&self, directory: &ResolvedPath) -> bool {
        self.watches
            .contains_key(&directory.as_path().to_string_lossy().into_owned())
    }

    /// Every watch, for release when the workspace closes.
    pub fn drain_all(&mut self) -> Vec<WatchId> {
        let ids = self.watches.values().map(|w| w.id).collect();
        self.watches.clear();
        ids
    }
}

/// What the kernel said, before anything decided whether it is worth saying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawKind {
    Created,
    Modified,
    Deleted,
    /// Half of a move. The cookie pairs the two halves; an unpaired half at flush time is a
    /// deletion (moved out) or a creation (moved in), which is the correct classification
    /// rather than a degraded one.
    MovedFrom(u32),
    MovedTo(u32),
    /// The kernel's own queue overflowed and events were lost. Reported rather than dropped:
    /// a watcher that is running and silent is the one outcome the watch requirements forbid.
    Overflow,
}

/// One observation, workspace-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: RawKind,
    pub relative_path: String,
    pub is_directory: bool,
    pub size: u64,
    /// Unix seconds, the unit a listing already uses.
    pub modified: i64,
}

/// What crosses the wire, after coalescing. Carries no content -- no bytes and no hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Created,
    Modified,
    Deleted,
    Renamed { to: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEvent {
    pub kind: EventKind,
    pub relative_path: String,
    pub is_directory: bool,
    pub size: u64,
    pub modified: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn resolved(p: &str) -> ResolvedPath {
        ResolvedPath::for_test(PathBuf::from(p))
    }

    #[test]
    fn adding_a_directory_twice_yields_one_watch() {
        let mut set = WatchSet::new();
        let (first, fresh) = set.acquire(resolved("/ws/src"));
        assert!(fresh, "the first acquire must ask the host");
        let (second, again) = set.acquire(resolved("/ws/src"));
        assert_eq!(first, second, "one directory is one watch");
        assert!(!again, "the second must not ask the host again");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn a_watch_survives_until_its_last_reason_goes() {
        // A folder expanded and a file open inside it are two reasons for one directory.
        // Collapsing the folder must not stop reporting the open file (FR-003c, FR-004).
        let mut set = WatchSet::new();
        set.acquire(resolved("/ws/src"));
        set.acquire(resolved("/ws/src"));
        assert_eq!(
            set.release(&resolved("/ws/src")),
            None,
            "one reason remains"
        );
        assert!(set.contains(&resolved("/ws/src")));
        assert!(
            set.release(&resolved("/ws/src")).is_some(),
            "the last reason releases it"
        );
        assert!(set.is_empty());
    }

    #[test]
    fn releasing_something_unheld_is_not_an_error() {
        let mut set = WatchSet::new();
        assert_eq!(set.release(&resolved("/ws/never")), None);
    }

    #[test]
    fn draining_returns_every_watch_once() {
        let mut set = WatchSet::new();
        set.acquire(resolved("/ws/a"));
        set.acquire(resolved("/ws/a")); // two reasons, still one watch
        set.acquire(resolved("/ws/b"));
        let ids = set.drain_all();
        assert_eq!(ids.len(), 2, "closing a workspace releases each watch once");
        assert!(set.is_empty());
    }
}
