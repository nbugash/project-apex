//! What the client does with a change it was told about.
//!
//! Two rules govern everything here. Every path is re-validated at this end regardless of what
//! the engine checked (FR-014, Principle VI) -- a boundary enforced on one side only is a
//! boundary enforced nowhere. And nothing here ever marks content **valid**: validity is a hash
//! comparison and an event carries no hash, so an event can only ever cast doubt (FR-019).

use crate::application::ports::workspace_cache::WorkspaceCache;
use crate::domain::workspace::{RelPath, WorkspaceId};

/// One change, as the client sees it after the wire types are translated away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Created {
        is_directory: bool,
        size: u64,
        modified: i64,
    },
    Modified {
        is_directory: bool,
        size: u64,
        modified: i64,
    },
    Deleted,
    Renamed {
        to: RelPath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEvent {
    pub path: RelPath,
    pub change: Change,
}

/// What an event did, so a caller can assert on it rather than on a side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyReport {
    pub applied: usize,
    /// Paths the client itself refused. Never zero silently: SC-010 asserts this number.
    pub refused: usize,
    pub marked_unproven: usize,
    pub rows_renamed: usize,
}

pub struct ApplyFileEvent<'a> {
    cache: &'a dyn WorkspaceCache,
}

impl<'a> ApplyFileEvent<'a> {
    pub fn new(cache: &'a dyn WorkspaceCache) -> Self {
        Self { cache }
    }

    /// Apply a batch. One frame is one batch (A-COALESCE), so this is the unit of work.
    pub fn apply(&self, ws: &WorkspaceId, events: &[FileEvent]) -> ApplyReport {
        let mut report = ApplyReport::default();
        for event in events {
            // `RelPath` cannot hold an escaping path -- it is parsed, not trusted -- so an
            // event that got this far is contained. What is checked here is the destination of
            // a rename, which arrives as a second path and would otherwise be applied
            // unchecked.
            if let Change::Renamed { to } = &event.change {
                if to.as_str().is_empty() {
                    report.refused += 1;
                    continue;
                }
            }
            match &event.change {
                Change::Created { .. } | Change::Modified { .. } => {
                    // An event never marks content valid, and never fetches a path the client
                    // has not fetched (FR-019, FR-020). Casting doubt is all it can do.
                    if self.cache.mark_unproven(ws, &event.path).is_ok() {
                        report.marked_unproven += 1;
                    }
                    report.applied += 1;
                }
                Change::Deleted => {
                    let _ = self.cache.mark_stale(ws, &event.path);
                    report.applied += 1;
                }
                Change::Renamed { to } => {
                    // The entry moves; it is not lost and re-found. That is what lets the
                    // cached content survive (FR-021, FR-022).
                    if let Ok(rows) = self.cache.rename_subtree(ws, &event.path, to) {
                        report.rows_renamed += rows;
                    }
                    report.applied += 1;
                }
            }
        }
        report
    }

    /// A wholesale invalidation (§10.4, FR-017, FR-018).
    ///
    /// Marks the tree stale, discards no blob, fetches nothing -- and marks every **open tab**
    /// unproven. The bulk rule discards the individual events, so without that last part a
    /// branch switch that rewrote a file the developer has open would report nothing about it,
    /// and FR-023 admits no exception for how a change arrived (FR-023b, SC-004a).
    pub fn invalidate_all(&self, ws: &WorkspaceId, open_tabs: &[RelPath]) -> ApplyReport {
        let mut report = ApplyReport::default();
        let _ = self.cache.mark_stale(ws, &RelPath::root());
        report.applied += 1;
        for tab in open_tabs {
            if self.cache.mark_unproven(ws, tab).is_ok() {
                report.marked_unproven += 1;
            }
        }
        report
    }
}
