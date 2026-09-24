//! An in-memory `FileWatcher`, with no filesystem behind it.
//!
//! Everything that decides anything in this feature sits above the port, so everything that
//! decides anything can be tested against this. Capacity exhaustion is a field rather than a
//! kernel limit, which is what makes the exhaustion case a test rather than a root-only one.

use apex_engine::application::ports::clock::Millis;
use apex_engine::application::ports::file_watcher::{FileWatcher, WatchError};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::watch::{RawEvent, WatchId};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct FakeWatcher {
    held: BTreeSet<u64>,
    next: u64,
    queued: Vec<RawEvent>,
    /// `None` means unlimited.
    capacity: Option<usize>,
    not_directories: BTreeSet<String>,
    /// Every timeout `poll` was asked to wait for, so a test can assert the thread waits for
    /// what the coalescer said rather than for a number of its own.
    pub polled_for: Vec<Millis>,
}

impl FakeWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// The host will hold at most `n` watches; the next request is refused.
    pub fn with_capacity(n: usize) -> Self {
        Self {
            capacity: Some(n),
            ..Self::default()
        }
    }

    pub fn feed(&mut self, event: RawEvent) {
        self.queued.push(event);
    }

    pub fn refuse_as_file(&mut self, path: &str) {
        self.not_directories.insert(path.to_string());
    }
}

impl FileWatcher for FakeWatcher {
    fn watch(&mut self, directory: &ResolvedPath) -> Result<WatchId, WatchError> {
        let shown = directory.as_path().to_string_lossy().into_owned();
        if self.not_directories.contains(&shown) {
            return Err(WatchError::NotADirectory);
        }
        if self.capacity.is_some_and(|c| self.held.len() >= c) {
            return Err(WatchError::CapacityExhausted);
        }
        self.next += 1;
        self.held.insert(self.next);
        Ok(WatchId(self.next))
    }

    fn unwatch(&mut self, id: WatchId) -> Result<(), WatchError> {
        // Releasing something already released is not an error: that is what makes a client's
        // re-sent set idempotent rather than a sequence to get exactly right.
        self.held.remove(&id.0);
        Ok(())
    }

    fn poll(&mut self, timeout: Millis) -> Vec<RawEvent> {
        self.polled_for.push(timeout);
        std::mem::take(&mut self.queued)
    }

    fn held(&self) -> usize {
        self.held.len()
    }
}
