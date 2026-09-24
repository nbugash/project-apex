//! The port's own guarantees (contracts/watcher-port.md).

mod common;

use apex_engine::application::ports::file_watcher::{FileWatcher, WatchError};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::watch::{RawEvent, RawKind};
use common::fake_watcher::FakeWatcher;
use std::path::PathBuf;

fn dir(p: &str) -> ResolvedPath {
    ResolvedPath::resolve_absolute_for_watch(&test_root(), p)
}

fn test_root() -> apex_engine::domain::path::CanonicalRoot {
    // A root the debug assertion accepts; every path below is under it.
    use apex_engine::application::ports::file_system::FileSystem;
    let fs = common::FakeFileSystem::new();
    fs.dir("/ws");
    ResolvedPath::canonical_root(std::path::Path::new("/ws"), &fs as &dyn FileSystem).expect("root")
}

#[test]
fn poll_never_blocks_longer_than_its_timeout() {
    // W5, and the mechanism behind SC-001. The coalescer computes when its next window closes
    // and asks for exactly that long; a port that over-blocks delays every pending event, and
    // no test asserting on event *content* would notice.
    let mut w = FakeWatcher::new();
    let started = std::time::Instant::now();
    let out = w.poll(40);
    assert!(out.is_empty());
    assert!(
        started.elapsed() < std::time::Duration::from_millis(400),
        "poll took {:?}, which is an order of magnitude past what it was asked for",
        started.elapsed()
    );
    assert_eq!(
        w.polled_for,
        vec![40],
        "the timeout it was asked for is the one it used"
    );
}

#[test]
fn a_refusal_for_one_directory_leaves_every_other_watch_intact() {
    // W2 and FR-005a. A host that cannot hold another watch must not lose the ones it has.
    let mut w = FakeWatcher::with_capacity(2);
    let first = w.watch(&dir("/ws/a")).expect("first fits");
    let second = w.watch(&dir("/ws/b")).expect("second fits");
    assert_eq!(w.watch(&dir("/ws/c")), Err(WatchError::CapacityExhausted));
    assert_eq!(w.held(), 2, "the refusal took nothing away");
    assert!(w.unwatch(first).is_ok());
    assert!(w.watch(&dir("/ws/c")).is_ok(), "room freed is room usable");
    let _ = second;
}

#[test]
fn releasing_twice_is_not_an_error() {
    // What makes a re-sent set idempotent rather than a sequence to get exactly right.
    let mut w = FakeWatcher::new();
    let id = w.watch(&dir("/ws/a")).expect("watch");
    assert!(w.unwatch(id).is_ok());
    assert!(w.unwatch(id).is_ok(), "already released is success");
    assert_eq!(w.held(), 0);
}

#[test]
fn the_port_yields_no_content_and_opens_no_file() {
    // W6 and FR-013. Structural: there is no field on a RawEvent that could carry bytes or a
    // hash, so a client cannot mark a blob valid from one.
    let mut w = FakeWatcher::new();
    w.feed(RawEvent {
        kind: RawKind::Modified,
        relative_path: "a.rs".into(),
        is_directory: false,
        size: 10,
        modified: 5,
    });
    let out = w.poll(0);
    let rendered = format!("{:?}", out[0]);
    for forbidden in ["sha", "hash", "content", "bytes"] {
        assert!(
            !rendered.contains(forbidden),
            "{forbidden} reached a raw event: {rendered}"
        );
    }
}

#[test]
fn held_counts_host_watches_not_requests() {
    // SC-009/009a/009b assert through this rather than reading /proc, which would make them
    // Linux-only and root-dependent.
    let mut w = FakeWatcher::new();
    assert_eq!(w.held(), 0);
    w.watch(&dir("/ws/a")).expect("watch");
    w.watch(&dir("/ws/b")).expect("watch");
    assert_eq!(w.held(), 2);
}

#[test]
fn a_path_that_is_not_a_directory_is_refused_as_such() {
    let mut w = FakeWatcher::new();
    w.refuse_as_file("/ws/a.rs");
    assert_eq!(w.watch(&dir("/ws/a.rs")), Err(WatchError::NotADirectory));
}

#[test]
fn an_overflow_is_reported_rather_than_dropped() {
    // W7. The kernel drops events and says so once; everything dropped is a change the client
    // would otherwise never hear about.
    let mut w = FakeWatcher::new();
    w.feed(RawEvent {
        kind: RawKind::Overflow,
        relative_path: String::new(),
        is_directory: false,
        size: 0,
        modified: 0,
    });
    assert_eq!(w.poll(0)[0].kind, RawKind::Overflow);
    let _ = PathBuf::new();
}
