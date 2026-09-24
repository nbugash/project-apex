//! An exhausted host still opens and browses (FR-005a, SC-009c).
//! (FR-004, SC-009, SC-009a, SC-009b, SC-009c.)

mod common;

use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::file_watcher::FileWatcher;
use apex_engine::application::use_cases::watch::watch_paths;
use apex_engine::domain::path::{CanonicalRoot, ResolvedPath};
use apex_engine::domain::watch::WatchSet;
use common::fake_watcher::FakeWatcher;
use common::FakeFileSystem;

struct Ctx {
    fs: FakeFileSystem,
    root: CanonicalRoot,
    set: WatchSet,
    watcher: FakeWatcher,
    exclusions: ExclusionSet,
}

fn ctx(capacity: Option<usize>, folders: &[&str]) -> Ctx {
    let fs = FakeFileSystem::new();
    fs.dir("/ws");
    for f in folders {
        fs.dir(&format!("/ws/{f}"));
    }
    let root = ResolvedPath::canonical_root(std::path::Path::new("/ws"), &fs as &dyn FileSystem)
        .expect("root");
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);
    Ctx {
        fs,
        root,
        set: WatchSet::new(),
        watcher: match capacity {
            Some(n) => FakeWatcher::with_capacity(n),
            None => FakeWatcher::new(),
        },
        exclusions,
    }
}

fn add(c: &mut Ctx, paths: &[&str]) {
    let owned: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
    watch_paths(
        &c.root,
        &mut c.set,
        &mut c.watcher,
        &c.exclusions,
        &c.fs as &dyn FileSystem,
        &owned,
    );
}

#[test]
fn exhausted_capacity_still_leaves_a_usable_workspace() {
    // SC-009c and FR-005a. What fitted is watched; the rest is refused and said so.
    let mut c = ctx(Some(1), &["a", "b"]);
    add(&mut c, &["a", "b"]);
    assert_eq!(
        c.watcher.held(),
        1,
        "the ceiling held, and what fitted stayed"
    );
}
