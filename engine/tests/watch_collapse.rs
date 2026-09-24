//! Collapsing releases, unless something still needs it (FR-003a, SC-009b).
//! (FR-004, SC-009, SC-009a, SC-009b, SC-009c.)

mod common;

use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::file_watcher::FileWatcher;
use apex_engine::application::use_cases::watch::{unwatch_paths, watch_paths};
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

fn remove(c: &mut Ctx, paths: &[&str]) {
    let owned: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
    unwatch_paths(
        &c.root,
        &mut c.set,
        &mut c.watcher,
        &c.fs as &dyn FileSystem,
        &owned,
    );
}

#[test]
fn collapsing_a_folder_releases_its_watch() {
    let mut c = ctx(None, &["a", "b"]);
    add(&mut c, &["a", "b"]);
    let expanded = c.watcher.held();
    remove(&mut c, &["b"]);
    assert!(c.watcher.held() < expanded, "collapsing released something");
    remove(&mut c, &["a"]);
    assert_eq!(
        c.watcher.held(),
        0,
        "the root goes with the last reason for it"
    );
}
