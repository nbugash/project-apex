//! The one exclusion set: `.gitignore` plus §10.3's fixed list (FR-006, FR-009, A-IGNORE).

mod common;

use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::domain::path::ResolvedPath;
use common::FakeFileSystem;

fn resolve(fs: &FakeFileSystem) -> ExclusionSet {
    let root = ResolvedPath::canonical_root(std::path::Path::new("/ws"), fs as &dyn FileSystem)
        .expect("root resolves");
    ExclusionSet::resolve(&root, fs as &dyn FileSystem)
}

#[test]
fn the_built_in_set_applies_with_no_gitignore_at_all() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws").dir("/ws/node_modules").dir("/ws/src");
    fs.file("/ws/src/main.rs", b"fn main() {}");

    let set = resolve(&fs);
    for excluded in [
        "node_modules",
        ".git",
        "target",
        "dist",
        "build",
        ".venv",
        "__pycache__",
    ] {
        assert!(
            set.is_excluded(excluded, true),
            "{excluded} is in §10.3's fixed list"
        );
    }
    assert!(!set.is_excluded("src", true));
    assert!(!set.is_excluded("src/main.rs", false));
}

#[test]
fn the_built_in_set_excludes_everything_beneath_it() {
    // The cost that matters is not the directory, it is the churn inside it during an install.
    let fs = FakeFileSystem::new();
    fs.dir("/ws")
        .dir("/ws/node_modules")
        .dir("/ws/node_modules/left-pad");
    fs.file("/ws/node_modules/left-pad/index.js", b"");

    let set = resolve(&fs);
    assert!(set.is_excluded("node_modules/left-pad/index.js", false));
    assert!(set.is_excluded("node_modules/left-pad", true));
}

#[test]
fn it_applies_at_any_depth_not_only_the_root() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws")
        .dir("/ws/crates")
        .dir("/ws/crates/a")
        .dir("/ws/crates/a/target");
    fs.file("/ws/crates/a/target/out.bin", b"");

    let set = resolve(&fs);
    assert!(
        set.is_excluded("crates/a/target", true),
        "a nested build directory still churns"
    );
    assert!(set.is_excluded("crates/a/target/out.bin", false));
}

#[test]
fn a_root_gitignore_is_read() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws").dir("/ws/src");
    fs.file("/ws/.gitignore", b"*.log\n# a comment\n\nscratch/\n");
    fs.file("/ws/app.log", b"");

    let set = resolve(&fs);
    assert!(set.is_excluded("app.log", false));
    assert!(
        set.is_excluded("src/debug.log", false),
        "unanchored patterns match at any depth"
    );
    assert!(set.is_excluded("scratch", true));
    assert!(!set.is_excluded("src/main.rs", false));
}

#[test]
fn a_nested_gitignore_applies_only_beneath_itself() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws").dir("/ws/docs").dir("/ws/src");
    fs.file("/ws/docs/.gitignore", b"*.draft\n");
    fs.file("/ws/docs/notes.draft", b"");
    fs.file("/ws/src/notes.draft", b"");

    let set = resolve(&fs);
    assert!(set.is_excluded("docs/notes.draft", false));
    assert!(
        !set.is_excluded("src/notes.draft", false),
        "a nested .gitignore must not reach outside its own directory"
    );
}

#[test]
fn a_negation_reinstates_what_an_earlier_pattern_excluded() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws");
    fs.file("/ws/.gitignore", b"*.log\n!keep.log\n");

    let set = resolve(&fs);
    assert!(set.is_excluded("app.log", false));
    assert!(!set.is_excluded("keep.log", false), "a later negation wins");
}

#[test]
fn an_anchored_pattern_does_not_match_at_depth() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws")
        .dir("/ws/src")
        .dir("/ws/src/build")
        .dir("/ws/out");
    fs.file("/ws/.gitignore", b"/out\n");

    let set = resolve(&fs);
    assert!(set.is_excluded("out", true));
    assert!(
        !set.is_excluded("src/out", true),
        "a leading slash anchors to the root"
    );
}

#[test]
fn a_single_star_does_not_cross_a_separator() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws").dir("/ws/a").dir("/ws/a/b");
    fs.file("/ws/.gitignore", b"/a/*.rs\n");

    let set = resolve(&fs);
    assert!(set.is_excluded("a/x.rs", false));
    assert!(
        !set.is_excluded("a/b/x.rs", false),
        "* stops at a segment boundary"
    );
}

#[test]
fn a_double_star_does_cross_separators() {
    let fs = FakeFileSystem::new();
    fs.dir("/ws").dir("/ws/a").dir("/ws/a/b");
    fs.file("/ws/.gitignore", b"/a/**/x.rs\n");

    let set = resolve(&fs);
    assert!(set.is_excluded("a/b/x.rs", false));
    assert!(set.is_excluded("a/b/c/x.rs", false));
}

#[test]
fn the_walk_does_not_descend_into_what_it_has_already_excluded() {
    // A .gitignore inside an excluded directory must never be read: reading it means the walk
    // went in, and going in is the traversal this design exists to avoid on a large install.
    let fs = FakeFileSystem::new();
    fs.dir("/ws")
        .dir("/ws/node_modules")
        .dir("/ws/node_modules/pkg");
    fs.file("/ws/node_modules/pkg/.gitignore", b"!node_modules\nsrc\n");
    fs.dir("/ws/src");

    let set = resolve(&fs);
    assert!(set.is_excluded("node_modules", true), "still excluded");
    assert!(
        !set.is_excluded("src", true),
        "a pattern from inside an excluded directory was read; the walk descended"
    );
}

#[test]
fn there_is_no_per_workspace_configuration() {
    // FR-009: the only inputs are the repository's own files and the fixed list. A set built
    // twice from the same tree is the same set, with nothing else able to influence it.
    let fs = FakeFileSystem::new();
    fs.dir("/ws");
    fs.file("/ws/.gitignore", b"*.tmp\n");
    let first = resolve(&fs);
    let second = resolve(&fs);
    assert_eq!(first.len(), second.len());
    assert!(first.is_excluded("x.tmp", false) && second.is_excluded("x.tmp", false));
}
