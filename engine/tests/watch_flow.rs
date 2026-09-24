//! The engine's watch path end to end, with no filesystem and no kernel.
//!
//! A fake watcher, a fake clock and the real coalescer, use case and exclusion set. Everything
//! that decides anything is here; the only thing missing is the library that does not decide.

mod common;

use apex_engine::application::coalescer::{Coalescer, Emission};
use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::clock::Clock;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::file_watcher::FileWatcher;
use apex_engine::application::use_cases::watch::{unwatch_paths, watch_paths};
use apex_engine::domain::path::{CanonicalRoot, ResolvedPath};
use apex_engine::domain::watch::{EventKind, RawEvent, RawKind, WatchSet};
use apex_protocol::wire::RefusalReason;
use common::fake_clock::FakeClock;
use common::fake_watcher::FakeWatcher;
use common::FakeFileSystem;

fn tree() -> FakeFileSystem {
    let fs = FakeFileSystem::new();
    fs.dir("/ws")
        .dir("/ws/src")
        .dir("/ws/src/parser")
        .dir("/ws/node_modules");
    fs.file("/ws/src/main.rs", b"fn main() {}");
    fs.file("/ws/src/parser/expr.rs", b"");
    fs
}

fn root(fs: &FakeFileSystem) -> CanonicalRoot {
    ResolvedPath::canonical_root(std::path::Path::new("/ws"), fs as &dyn FileSystem).expect("root")
}

fn raw(kind: RawKind, path: &str) -> RawEvent {
    RawEvent {
        kind,
        relative_path: path.into(),
        is_directory: false,
        size: 7,
        modified: 11,
    }
}

#[test]
fn expanding_a_folder_watches_it_and_its_ancestors() {
    // FR-003. Ancestors because a rename of an ancestor changes every path beneath it and no
    // descendant watch would see it.
    let fs = tree();
    let root = root(&fs);
    let mut set = WatchSet::new();
    let mut watcher = FakeWatcher::new();
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);

    let result = watch_paths(
        &root,
        &mut set,
        &mut watcher,
        &exclusions,
        &fs as &dyn FileSystem,
        &["src/parser".to_string()],
    );

    assert!(result.refused.is_empty(), "{:?}", result.refused);
    // src/parser, src, and the root itself.
    assert_eq!(watcher.held(), 3, "the folder and its ancestors");
}

#[test]
fn a_file_path_watches_its_parent_not_the_file() {
    // FR-003c. An open tab contributes its containing directory, which is what inotify
    // observes, and what makes a sibling's creation visible too.
    let fs = tree();
    let root = root(&fs);
    let mut set = WatchSet::new();
    let mut watcher = FakeWatcher::new();
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);

    watch_paths(
        &root,
        &mut set,
        &mut watcher,
        &exclusions,
        &fs as &dyn FileSystem,
        &["src/main.rs".to_string()],
    );
    assert_eq!(watcher.held(), 2, "src and the root, not the file");
}

#[test]
fn collapsing_a_folder_that_still_holds_an_open_file_releases_nothing_it_needs() {
    // FR-003c and FR-004 from two directions, and SC-009b. A folder holding an open file is one
    // directory for two reasons; the collapse drops one reason and the watch must survive.
    let fs = tree();
    let root = root(&fs);
    let mut set = WatchSet::new();
    let mut watcher = FakeWatcher::new();
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);
    let paths = ["src".to_string(), "src/main.rs".to_string()];

    watch_paths(
        &root,
        &mut set,
        &mut watcher,
        &exclusions,
        &fs as &dyn FileSystem,
        &paths,
    );
    let before = watcher.held();

    unwatch_paths(
        &root,
        &mut set,
        &mut watcher,
        &fs as &dyn FileSystem,
        &["src".to_string()],
    );

    assert_eq!(
        watcher.held(),
        before,
        "the open file still needs src watched; collapsing the folder must not take it away"
    );

    unwatch_paths(
        &root,
        &mut set,
        &mut watcher,
        &fs as &dyn FileSystem,
        &["src/main.rs".to_string()],
    );
    assert_eq!(watcher.held(), 0, "the last reason releases it");
}

#[test]
fn an_excluded_path_is_refused_rather_than_silently_skipped() {
    // FR-008. Silence is indistinguishable from a working watch, which is the one outcome
    // FR-005 forbids.
    let fs = tree();
    let root = root(&fs);
    let mut set = WatchSet::new();
    let mut watcher = FakeWatcher::new();
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);

    let result = watch_paths(
        &root,
        &mut set,
        &mut watcher,
        &exclusions,
        &fs as &dyn FileSystem,
        &["node_modules".to_string()],
    );
    assert_eq!(result.refused.len(), 1);
    assert_eq!(result.refused[0].reason, RefusalReason::Excluded);
    assert_eq!(watcher.held(), 0, "nothing was watched");
}

#[test]
fn exhausted_capacity_is_reported_and_the_workspace_stays_usable() {
    // FR-005, FR-005a, SC-009c. A refusal is data; the call succeeds.
    let fs = tree();
    let root = root(&fs);
    let mut set = WatchSet::new();
    let mut watcher = FakeWatcher::with_capacity(1);
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);

    let result = watch_paths(
        &root,
        &mut set,
        &mut watcher,
        &exclusions,
        &fs as &dyn FileSystem,
        &["src/parser".to_string()],
    );
    assert_eq!(result.refused.len(), 1, "told, not silent");
    assert_eq!(result.refused[0].reason, RefusalReason::Capacity);
    assert_eq!(watcher.held(), 1, "what fitted is still watched");
}

#[test]
fn a_created_file_arrives_with_the_metadata_a_row_needs() {
    // FR-013a and US1 acceptance 1. Without the metadata the client cannot place the file in
    // the tree at all: three columns are NOT NULL and the only alternative is asking about a
    // path it was just told about, which FR-020 forbids.
    let clock = FakeClock::new();
    let mut watcher = FakeWatcher::new();
    watcher.feed(raw(RawKind::Created, "src/token.rs"));
    let mut coalescer = Coalescer::default();

    for event in watcher.poll(0) {
        coalescer.accept(event, clock.now());
    }
    clock.advance(150);
    let Emission::Batch(out) = coalescer.drain_due(clock.now()) else {
        panic!("a single creation is not a wholesale invalidation");
    };
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, EventKind::Created);
    assert_eq!(out[0].size, 7);
    assert_eq!(out[0].modified, 11);
}

#[test]
fn a_deleted_file_is_reported_as_deleted() {
    let clock = FakeClock::new();
    let mut coalescer = Coalescer::default();
    coalescer.accept(raw(RawKind::Deleted, "src/old.rs"), clock.now());
    clock.advance(150);
    let Emission::Batch(out) = coalescer.drain_due(clock.now()) else {
        panic!("expected a batch");
    };
    assert_eq!(out[0].kind, EventKind::Deleted);
}
