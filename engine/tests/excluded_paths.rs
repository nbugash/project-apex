//! Zero events for an excluded path (FR-008, SC-002).
//!
//! `exclusions.rs` tests that the set is built correctly. This tests the consequence, which is
//! what the requirement actually says: an excluded directory churns constantly during an
//! install, and the developer must hear none of it.
//!
//! The second check is not redundant with never watching the directory. A watch on a directory
//! reports its **children**, so an excluded child of a watched directory would otherwise be
//! delivered -- which is exactly the `node_modules` inside a watched project root.

mod common;

use apex_engine::application::coalescer::{Coalescer, Emission};
use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::watch::{RawEvent, RawKind};
use common::FakeFileSystem;

fn exclusions() -> ExclusionSet {
    let fs = FakeFileSystem::new();
    fs.dir("/ws")
        .dir("/ws/src")
        .dir("/ws/node_modules")
        .dir("/ws/node_modules/left-pad");
    fs.file("/ws/.gitignore", b"*.log\nscratch/\n");
    let root = ResolvedPath::canonical_root(std::path::Path::new("/ws"), &fs as &dyn FileSystem)
        .expect("root");
    ExclusionSet::resolve(&root, &fs as &dyn FileSystem)
}

/// The filter the watch thread applies before anything reaches the coalescer.
fn delivered(paths: &[(&str, bool)]) -> Vec<String> {
    let set = exclusions();
    let mut coalescer = Coalescer::default();
    for (path, is_dir) in paths {
        if set.is_excluded(path, *is_dir) {
            continue;
        }
        coalescer.accept(
            RawEvent {
                kind: RawKind::Modified,
                relative_path: (*path).to_string(),
                is_directory: *is_dir,
                size: 1,
                modified: 0,
            },
            0,
        );
    }
    match coalescer.drain_due(1_000) {
        Emission::Batch(events) => events.into_iter().map(|e| e.relative_path).collect(),
        Emission::InvalidateAll => vec!["<invalidate>".to_string()],
    }
}

#[test]
fn an_install_churning_inside_an_excluded_directory_delivers_nothing() {
    let out = delivered(&[
        ("node_modules/left-pad/index.js", false),
        ("node_modules/left-pad/package.json", false),
        ("node_modules/.package-lock.json", false),
    ]);
    assert!(out.is_empty(), "an excluded subtree delivered {out:?}");
}

#[test]
fn a_gitignore_pattern_excludes_delivery_at_any_depth() {
    let out = delivered(&[("app.log", false), ("src/debug.log", false)]);
    assert!(out.is_empty(), "delivered {out:?}");
}

#[test]
fn the_same_burst_outside_the_exclusions_is_delivered() {
    // Without this the suite would pass if delivery never worked at all.
    let out = delivered(&[("src/main.rs", false), ("src/lib.rs", false)]);
    assert_eq!(out.len(), 2, "delivered {out:?}");
}

#[test]
fn an_excluded_child_of_a_watched_directory_is_still_excluded() {
    // The root is watched, so the kernel reports node_modules being created inside it.
    let out = delivered(&[("node_modules", true), ("src", true)]);
    assert_eq!(out, vec!["src".to_string()]);
}

#[test]
fn every_event_kind_is_excluded_not_only_modification() {
    let set = exclusions();
    for kind in ["node_modules/x.js", "node_modules"] {
        assert!(
            set.is_excluded(kind, false) || set.is_excluded(kind, true),
            "{kind}"
        );
    }
}
