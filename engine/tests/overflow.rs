//! A kernel queue overflow is a wholesale invalidation (A-COALESCE, FR-005, W7).
//!
//! The queue is finite. When a burst outruns the reader the kernel drops events and says so
//! once, and everything dropped is a change the client would otherwise never hear about --
//! precisely the silence the watch requirements forbid. The client's response to an
//! invalidation is already correct for it: mark stale, re-read lazily, discard nothing.

use apex_engine::application::coalescer::{Coalescer, Emission};
use apex_engine::domain::watch::{RawEvent, RawKind};

fn event(kind: RawKind, path: &str) -> RawEvent {
    RawEvent {
        kind,
        relative_path: path.into(),
        is_directory: false,
        size: 1,
        modified: 0,
    }
}

#[test]
fn an_overflow_becomes_an_invalidation() {
    let mut c = Coalescer::default();
    c.accept(event(RawKind::Overflow, ""), 0);
    assert_eq!(c.drain_due(0), Emission::InvalidateAll);
}

#[test]
fn an_overflow_supersedes_whatever_was_pending() {
    // The pending events describe a world the overflow has just admitted we cannot see.
    // Delivering them as well would report a fragment as though it were the whole change.
    let mut c = Coalescer::default();
    c.accept(event(RawKind::Modified, "a.rs"), 0);
    c.accept(event(RawKind::Modified, "b.rs"), 0);
    c.accept(event(RawKind::Overflow, ""), 1);
    assert_eq!(c.drain_due(200), Emission::InvalidateAll);
    // And nothing lingers to be delivered afterwards.
    assert_eq!(c.drain_due(400), Emission::Batch(vec![]));
}

#[test]
fn an_overflow_is_due_immediately() {
    // It must not wait out a coalescing window. The client is currently being told nothing,
    // and the window exists to reduce chatter rather than to delay bad news.
    let mut c = Coalescer::default();
    c.accept(event(RawKind::Overflow, ""), 0);
    assert_eq!(c.next_deadline(), Some(0));
}

#[test]
fn ordinary_events_still_flow_after_an_overflow() {
    let mut c = Coalescer::default();
    c.accept(event(RawKind::Overflow, ""), 0);
    assert_eq!(c.drain_due(0), Emission::InvalidateAll);
    c.accept(event(RawKind::Modified, "a.rs"), 10);
    let Emission::Batch(events) = c.drain_due(200) else {
        panic!("the watcher keeps working after an overflow");
    };
    assert_eq!(events.len(), 1);
}
