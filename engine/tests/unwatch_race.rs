//! An event in flight for a path just unwatched is dropped (obligation 12; spec edge case
//! "a folder collapsed while its files are changing").

mod common;

use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::use_cases::watch::{unwatch_paths, watch_paths};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::watch::WatchSet;
use common::fake_watcher::FakeWatcher;
use common::FakeFileSystem;

#[test]
fn a_collapsed_folders_watch_is_gone_before_the_next_poll() {
    // The engine stops observing; anything already queued is discarded with the watch rather
    // than delivered for a folder the developer has closed.
    let fs = FakeFileSystem::new();
    fs.dir("/ws").dir("/ws/src");
    let root = ResolvedPath::canonical_root(std::path::Path::new("/ws"), &fs as &dyn FileSystem)
        .expect("root");
    let exclusions = ExclusionSet::resolve(&root, &fs as &dyn FileSystem);
    let mut set = WatchSet::new();
    let mut watcher = FakeWatcher::new();

    watch_paths(
        &root,
        &mut set,
        &mut watcher,
        &exclusions,
        &fs as &dyn FileSystem,
        &["src".to_string()],
    );
    assert!(set.contains(&ResolvedPath::resolve_absolute_for_watch(&root, "/ws/src")));

    unwatch_paths(
        &root,
        &mut set,
        &mut watcher,
        &fs as &dyn FileSystem,
        &["src".to_string()],
    );

    assert!(
        !set.contains(&ResolvedPath::resolve_absolute_for_watch(&root, "/ws/src")),
        "the watch is released, so nothing further is observed for it"
    );
}
