//! Watches are returned when a workspace closes (FR-004, SC-009).
//! (FR-004, SC-009, SC-009a, SC-009b, SC-009c.)

mod common;

use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::file_watcher::FileWatcher;
use apex_engine::application::use_cases::watch::{release_all, watch_paths};
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
fn a_hundred_open_and_close_cycles_leak_nothing() {
    // SC-009 states it as a hundred cycles rather than one, and that is the point: a leak of a
    // single descriptor per cycle is invisible in one pass and unmistakable in a hundred.
    let mut c = ctx(None, &["a", "b", "c"]);
    let baseline = c.watcher.held();
    for _ in 0..100 {
        add(&mut c, &["a", "b", "c"]);
        release_all(&mut c.set, &mut c.watcher);
    }
    assert_eq!(
        c.watcher.held(),
        baseline,
        "a hundred cycles returned every watch"
    );
    assert!(c.set.is_empty());
}
#[test]
fn releasing_everything_twice_is_not_an_error() {
    let mut c = ctx(None, &["a"]);
    add(&mut c, &["a"]);
    release_all(&mut c.set, &mut c.watcher);
    release_all(&mut c.set, &mut c.watcher);
    assert_eq!(c.watcher.held(), 0);
}
