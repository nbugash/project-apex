//! The engine's half of Principle VI.
//!
//! The client re-validates every arriving path independently (`event_outside_root.rs` covers
//! that). This is the other half: a boundary enforced on one side only is a boundary enforced
//! nowhere, and FR-002 is an engine obligation.

mod common;

use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::file_watcher::FileWatcher;
use apex_engine::application::use_cases::watch::watch_paths;
use apex_engine::domain::path::{CanonicalRoot, ResolvedPath};
use apex_engine::domain::watch::WatchSet;
use common::fake_watcher::FakeWatcher;
use common::FakeFileSystem;

fn fixture() -> (FakeFileSystem, CanonicalRoot) {
    let fs = FakeFileSystem::new();
    fs.dir("/ws").dir("/ws/src").dir("/outside");
    fs.file("/outside/secret", b"");
    let root = ResolvedPath::canonical_root(std::path::Path::new("/ws"), &fs as &dyn FileSystem)
        .expect("root");
    (fs, root)
}

fn attempt(path: &str) -> (usize, usize) {
    let (fs, root) = fixture();
    let mut set = WatchSet::new();
    let mut watcher = FakeWatcher::new();
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);
    let result = watch_paths(
        &root,
        &mut set,
        &mut watcher,
        &exclusions,
        &fs as &dyn FileSystem,
        &[path.to_string()],
    );
    (watcher.held(), result.refused.len())
}

#[test]
fn a_path_that_climbs_out_of_the_root_watches_nothing() {
    let (held, _) = attempt("../outside");
    assert_eq!(held, 0, "nothing outside the root may be observed");
}

#[test]
fn an_absolute_path_watches_nothing() {
    let (held, _) = attempt("/outside/secret");
    assert_eq!(held, 0);
}

#[test]
fn a_path_with_a_null_byte_watches_nothing() {
    let (held, _) = attempt("src\0/../../outside");
    assert_eq!(held, 0);
}

#[test]
fn a_deeply_climbing_path_watches_nothing() {
    let (held, _) = attempt("src/../../../../../../etc");
    assert_eq!(held, 0);
}

#[test]
fn a_refusal_looks_the_same_whether_or_not_the_target_exists() {
    // FR-007's reasoning applied here: a caller able to tell the two apart could probe the
    // host's filesystem using nothing but refusals.
    let existing = attempt("../outside/secret");
    let absent = attempt("../outside/no-such-thing");
    assert_eq!(
        existing, absent,
        "a refusal must not leak what is out there"
    );
}

#[test]
fn a_contained_path_is_watched_normally() {
    // The other half of every negative test: without this the suite would pass if watching
    // never worked at all.
    let (held, refused) = attempt("src");
    assert!(held > 0, "a contained path must still be watched");
    assert_eq!(refused, 0);
}
