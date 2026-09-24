//! What is worth telling the client, and when.
//!
//! Pure. It is fed raw events and told the time; it opens no file, spawns no thread and
//! serialises nothing. That is what makes the volume requirements arithmetic: "a thousand
//! writes in one second" and "ten thousand changes at once" run instantly against a settable
//! counter, where the same assertions against a real clock would sleep and then be ignored.

use std::collections::BTreeMap;

use crate::application::ports::clock::Millis;
use crate::domain::watch::{EventKind, FileEvent, RawEvent, RawKind};

/// A-COALESCE. Repeated changes to one path collapse into one event per window.
///
/// Bounded above by the two-second reflection budget against a 250 ms round trip, and below by
/// having to collapse anything at all: an editor saves by writing a temporary file and renaming
/// it over the target, two or three events per save.
pub const WINDOW_MS: Millis = 100;

/// A-COALESCE. Distinct paths within `BULK_WINDOW_MS` before the whole tree is invalidated
/// instead. An order of magnitude above any human action, and a factor of eight inside
/// A-BULKSIZE once serialised.
pub const BULK_THRESHOLD: usize = 256;
pub const BULK_WINDOW_MS: Millis = 1_000;

/// What a flush produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Emission {
    /// One frame. Empty when nothing was due.
    Batch(Vec<FileEvent>),
    /// The tree is stale and the client re-reads lazily. Supersedes the individual events of
    /// the same flush rather than arriving alongside them.
    InvalidateAll,
}

#[derive(Debug, Clone)]
struct Pending {
    kind: EventKind,
    is_directory: bool,
    size: u64,
    modified: i64,
    /// When this path's window closes. Set once, by the **first** event for the path.
    due: Millis,
    /// Insertion order, so a batch is deterministic rather than whatever the map iterates.
    seq: u64,
}

#[derive(Debug)]
pub struct Coalescer {
    window: Millis,
    bulk_window: Millis,
    bulk_threshold: usize,
    pending: BTreeMap<String, Pending>,
    /// Halves of a move waiting for their partner, by inotify cookie.
    unpaired_from: BTreeMap<u32, (String, Millis)>,
    unpaired_to: BTreeMap<u32, (String, Millis, bool, u64, i64)>,
    /// Recent distinct-path sightings, for the bulk rule.
    recent: Vec<(Millis, String)>,
    overflowed: bool,
    seq: u64,
}

impl Default for Coalescer {
    fn default() -> Self {
        Self::new(WINDOW_MS, BULK_WINDOW_MS, BULK_THRESHOLD)
    }
}

impl Coalescer {
    pub fn new(window: Millis, bulk_window: Millis, bulk_threshold: usize) -> Self {
        Self {
            window,
            bulk_window,
            bulk_threshold,
            pending: BTreeMap::new(),
            unpaired_from: BTreeMap::new(),
            unpaired_to: BTreeMap::new(),
            recent: Vec::new(),
            overflowed: false,
            seq: 0,
        }
    }

    /// Absorb one observation. Emits nothing: emission is `drain_due`'s job, so that what is
    /// sent is a function of elapsed time rather than of arrival.
    pub fn accept(&mut self, raw: RawEvent, now: Millis) {
        if raw.kind == RawKind::Overflow {
            // The kernel dropped events and said so once. Everything dropped is a change the
            // client would otherwise never hear about -- the silence the watch requirements
            // forbid -- so it is the same answer §10.4 already gives to the same problem.
            self.overflowed = true;
            return;
        }
        self.note_recent(&raw.relative_path, now);

        match raw.kind {
            RawKind::MovedFrom(cookie) => {
                if let Some((to, seen, is_dir, size, modified)) = self.unpaired_to.remove(&cookie) {
                    // The window starts when the FIRST half was observed, not when the pair
                    // completed. Starting it at completion quietly lengthens the window for
                    // every rename, which is the one event kind where both halves are already
                    // in hand and there is nothing left to wait for.
                    self.record_rename(
                        raw.relative_path,
                        to,
                        is_dir,
                        size,
                        modified,
                        seen.min(now),
                    );
                } else {
                    self.unpaired_from.insert(cookie, (raw.relative_path, now));
                }
            }
            RawKind::MovedTo(cookie) => {
                if let Some((from, seen)) = self.unpaired_from.remove(&cookie) {
                    self.record_rename(
                        from,
                        raw.relative_path,
                        raw.is_directory,
                        raw.size,
                        raw.modified,
                        seen.min(now),
                    );
                } else {
                    self.unpaired_to.insert(
                        cookie,
                        (
                            raw.relative_path,
                            now,
                            raw.is_directory,
                            raw.size,
                            raw.modified,
                        ),
                    );
                }
            }
            RawKind::Created => self.record(raw, EventKind::Created, now),
            RawKind::Modified => self.record(raw, EventKind::Modified, now),
            RawKind::Deleted => self.record(raw, EventKind::Deleted, now),
            RawKind::Overflow => unreachable!("handled above"),
        }
    }

    fn record(&mut self, raw: RawEvent, kind: EventKind, now: Millis) {
        let seq = self.next_seq();
        let window = self.window;
        let entry = self
            .pending
            .entry(raw.relative_path.clone())
            .or_insert_with(|| Pending {
                kind: kind.clone(),
                is_directory: raw.is_directory,
                size: raw.size,
                modified: raw.modified,
                // Set once. A deadline reset by every arrival would mean a file written
                // continuously never reports at all -- bounded by elapsed time in the most
                // useless possible sense.
                due: now + window,
                seq,
            });
        entry.kind = merge(&entry.kind, &kind);
        // The trailing edge: the last write in a burst is the one whose state is sent,
        // because it is the one whose content the developer would fetch.
        entry.is_directory = raw.is_directory;
        entry.size = raw.size;
        entry.modified = raw.modified;
    }

    fn record_rename(
        &mut self,
        from: String,
        to: String,
        is_directory: bool,
        size: u64,
        modified: i64,
        started: Millis,
    ) {
        self.note_recent(&to, started);
        let seq = self.next_seq();
        let window = self.window;
        let entry = self.pending.entry(from).or_insert_with(|| Pending {
            kind: EventKind::Renamed { to: to.clone() },
            is_directory,
            size,
            modified,
            due: started + window,
            seq,
        });
        entry.kind = EventKind::Renamed { to };
        entry.is_directory = is_directory;
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn note_recent(&mut self, path: &str, now: Millis) {
        self.recent
            .retain(|(at, _)| now.saturating_sub(*at) < self.bulk_window);
        if !self.recent.iter().any(|(_, p)| p == path) {
            self.recent.push((now, path.to_string()));
        }
    }

    /// The earliest time at which `drain_due` could emit anything, or `None` when nothing
    /// pends. The watcher thread polls for exactly this long, which is what keeps timing in
    /// the pure component and leaves the thread with no policy of its own.
    pub fn next_deadline(&self) -> Option<Millis> {
        if self.overflowed {
            return Some(0);
        }
        let pending = self.pending.values().map(|p| p.due).min();
        let unpaired = self
            .unpaired_from
            .values()
            .map(|(_, at)| *at + self.window)
            .chain(
                self.unpaired_to
                    .values()
                    .map(|(_, at, ..)| *at + self.window),
            )
            .min();
        match (pending, unpaired) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Everything whose window has closed, as one frame.
    pub fn drain_due(&mut self, now: Millis) -> Emission {
        if self.overflowed {
            self.overflowed = false;
            self.pending.clear();
            self.unpaired_from.clear();
            self.unpaired_to.clear();
            self.recent.clear();
            return Emission::InvalidateAll;
        }

        self.recent
            .retain(|(at, _)| now.saturating_sub(*at) < self.bulk_window);
        if self.recent.len() >= self.bulk_threshold {
            self.pending.clear();
            self.unpaired_from.clear();
            self.unpaired_to.clear();
            self.recent.clear();
            return Emission::InvalidateAll;
        }

        self.expire_unpaired(now);

        let ready: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.due <= now)
            .map(|(path, _)| path.clone())
            .collect();

        let mut out: Vec<(u64, FileEvent)> = Vec::with_capacity(ready.len());
        for path in ready {
            let p = self.pending.remove(&path).expect("just listed");
            out.push((
                p.seq,
                FileEvent {
                    kind: p.kind,
                    relative_path: path,
                    is_directory: p.is_directory,
                    size: p.size,
                    modified: p.modified,
                },
            ));
        }
        out.sort_by_key(|(seq, _)| *seq);
        Emission::Batch(out.into_iter().map(|(_, e)| e).collect())
    }

    /// A move half whose partner never came is not a degraded rename. A file moved out of the
    /// workspace genuinely is a deletion from this workspace's point of view, and one moved in
    /// from outside genuinely is a creation.
    fn expire_unpaired(&mut self, now: Millis) {
        let window = self.window;
        let stale_from: Vec<u32> = self
            .unpaired_from
            .iter()
            .filter(|(_, (_, at))| at + window <= now)
            .map(|(c, _)| *c)
            .collect();
        for cookie in stale_from {
            let (path, at) = self.unpaired_from.remove(&cookie).expect("just listed");
            let seq = self.next_seq();
            self.pending.entry(path).or_insert(Pending {
                kind: EventKind::Deleted,
                is_directory: false,
                size: 0,
                modified: 0,
                due: at,
                seq,
            });
        }
        let stale_to: Vec<u32> = self
            .unpaired_to
            .iter()
            .filter(|(_, (_, at, ..))| at + window <= now)
            .map(|(c, _)| *c)
            .collect();
        for cookie in stale_to {
            let (path, at, is_dir, size, modified) =
                self.unpaired_to.remove(&cookie).expect("just listed");
            let seq = self.next_seq();
            self.pending.entry(path).or_insert(Pending {
                kind: EventKind::Created,
                is_directory: is_dir,
                size,
                modified,
                due: at,
                seq,
            });
        }
    }
}

/// A created file that is then written is still new to the client; a rename already names both
/// paths and nothing after it in the same window improves on that.
fn merge(existing: &EventKind, incoming: &EventKind) -> EventKind {
    match (existing, incoming) {
        (EventKind::Created, EventKind::Modified) => EventKind::Created,
        (EventKind::Renamed { to }, EventKind::Modified) => EventKind::Renamed { to: to.clone() },
        _ => incoming.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kind: RawKind, path: &str) -> RawEvent {
        RawEvent {
            kind,
            relative_path: path.into(),
            is_directory: false,
            size: 1,
            modified: 0,
        }
    }

    fn sized(kind: RawKind, path: &str, size: u64, modified: i64) -> RawEvent {
        RawEvent {
            kind,
            relative_path: path.into(),
            is_directory: false,
            size,
            modified,
        }
    }

    fn batch(e: Emission) -> Vec<FileEvent> {
        match e {
            Emission::Batch(v) => v,
            Emission::InvalidateAll => panic!("expected a batch, got an invalidation"),
        }
    }

    #[test]
    fn a_thousand_writes_in_one_second_are_bounded_by_time_not_by_writes() {
        // FR-012 and SC-007. The upper bound is the obvious half; the lower bound is the half
        // that matters. An assertion of "at most ten" passes most emphatically when the window
        // is infinite and the developer is told nothing at all, so both ends are asserted and
        // widening the window has to fail something.
        let mut c = Coalescer::default();
        let mut delivered = 0usize;
        for tick in 0..1000u64 {
            c.accept(raw(RawKind::Modified, "out/app.wasm"), tick);
            delivered += batch(c.drain_due(tick)).len();
        }
        delivered += batch(c.drain_due(1000)).len();

        assert!(
            delivered <= 10,
            "at most one per 100 ms window, got {delivered}"
        );
        assert!(
            delivered >= 1,
            "a thousand writes must not be reported as nothing"
        );
    }

    #[test]
    fn a_window_reports_the_last_write_not_the_first() {
        // The trailing edge is contractual. The last write is the one whose content the
        // developer would fetch, so it is the state that travels.
        let mut c = Coalescer::default();
        c.accept(sized(RawKind::Modified, "a.rs", 10, 100), 0);
        c.accept(sized(RawKind::Modified, "a.rs", 20, 200), 30);
        c.accept(sized(RawKind::Modified, "a.rs", 30, 300), 60);
        let out = batch(c.drain_due(100));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].size, 30, "the last write's state");
        assert_eq!(out[0].modified, 300);
    }

    #[test]
    fn nothing_is_emitted_before_the_window_closes() {
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::Created, "a.rs"), 0);
        assert!(
            batch(c.drain_due(99)).is_empty(),
            "99 ms is inside the window"
        );
        assert_eq!(batch(c.drain_due(100)).len(), 1);
    }

    #[test]
    fn the_next_deadline_is_what_the_thread_should_wait_for() {
        let mut c = Coalescer::default();
        assert_eq!(
            c.next_deadline(),
            None,
            "nothing pending, nothing to wait for"
        );
        c.accept(raw(RawKind::Modified, "a.rs"), 40);
        assert_eq!(c.next_deadline(), Some(140));
        c.accept(raw(RawKind::Modified, "b.rs"), 10);
        assert_eq!(c.next_deadline(), Some(110), "the earliest, not the latest");
    }

    #[test]
    fn a_created_file_that_is_then_written_is_still_created() {
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::Created, "new.rs"), 0);
        c.accept(raw(RawKind::Modified, "new.rs"), 10);
        let out = batch(c.drain_due(100));
        assert_eq!(
            out[0].kind,
            EventKind::Created,
            "the file is new to the client"
        );
    }

    #[test]
    fn a_rename_is_one_event_naming_both_paths() {
        // FR-011. Never a deletion followed by an unrelated creation: the projection survives
        // a rename only if the entry moves rather than being lost and re-found.
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::MovedFrom(7), "src/expr.rs"), 0);
        c.accept(raw(RawKind::MovedTo(7), "src/expression.rs"), 5);
        let out = batch(c.drain_due(100));
        assert_eq!(out.len(), 1, "one event, not two");
        assert_eq!(out[0].relative_path, "src/expr.rs");
        assert_eq!(
            out[0].kind,
            EventKind::Renamed {
                to: "src/expression.rs".into()
            }
        );
    }

    #[test]
    fn the_halves_of_a_move_pair_in_either_order() {
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::MovedTo(9), "b"), 0);
        c.accept(raw(RawKind::MovedFrom(9), "a"), 5);
        let out = batch(c.drain_due(100));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, EventKind::Renamed { to: "b".into() });
    }

    #[test]
    fn a_file_moved_out_of_the_workspace_is_a_deletion() {
        // Not a degraded rename. From this workspace's point of view the file is gone.
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::MovedFrom(3), "src/gone.rs"), 0);
        let out = batch(c.drain_due(200));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, EventKind::Deleted);
        assert_eq!(out[0].relative_path, "src/gone.rs");
    }

    #[test]
    fn a_file_moved_in_from_outside_is_a_creation() {
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::MovedTo(4), "src/arrived.rs"), 0);
        let out = batch(c.drain_due(200));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, EventKind::Created);
    }

    #[test]
    fn an_unpaired_half_is_not_classified_before_its_window_closes() {
        // Classifying eagerly would turn every rename into a delete-then-create, which is
        // exactly the shape FR-011 forbids.
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::MovedFrom(5), "a"), 0);
        assert!(
            batch(c.drain_due(50)).is_empty(),
            "its partner may still arrive"
        );
    }

    #[test]
    fn events_for_one_path_keep_the_order_they_occurred() {
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::Created, "a.rs"), 0);
        let first = batch(c.drain_due(100));
        c.accept(raw(RawKind::Deleted, "a.rs"), 150);
        let second = batch(c.drain_due(250));
        assert_eq!(first[0].kind, EventKind::Created);
        assert_eq!(
            second[0].kind,
            EventKind::Deleted,
            "later window, later event"
        );
    }

    #[test]
    fn a_batch_is_ordered_by_when_each_path_was_first_seen() {
        // Across paths no ordering is promised to the client, but the batch must be
        // deterministic or a test asserting on it is asserting on map iteration order.
        let mut c = Coalescer::default();
        c.accept(raw(RawKind::Modified, "z.rs"), 0);
        c.accept(raw(RawKind::Modified, "a.rs"), 1);
        c.accept(raw(RawKind::Modified, "m.rs"), 2);
        let out = batch(c.drain_due(200));
        let paths: Vec<&str> = out.iter().map(|e| e.relative_path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["z.rs", "a.rs", "m.rs"],
            "first seen, first sent"
        );
    }

    #[test]
    fn one_flush_is_one_frame() {
        let mut c = Coalescer::default();
        for i in 0..5u64 {
            c.accept(raw(RawKind::Modified, &format!("f{i}.rs")), i);
        }
        let out = batch(c.drain_due(200));
        assert_eq!(out.len(), 5, "five paths, one batch -- not five frames");
    }
}
