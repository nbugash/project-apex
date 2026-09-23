//! The fake is load-bearing for every use-case test, so it is tested itself. A double that
//! quietly disagrees with the real filesystem moves every failure it causes somewhere else.
mod common;
use apex_engine::application::ports::file_system::FileSystem;
use common::FakeFileSystem;
use std::path::Path;

#[test]
fn read_dir_returns_immediate_children_only() {
    let fs = FakeFileSystem::new();
    fs.file("/w/src/main.rs", b"x");
    fs.file("/w/src/deep/other.rs", b"y");
    fs.file("/w/top.txt", b"z");
    let mut names: Vec<String> = fs
        .read_dir(Path::new("/w"))
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    names.sort();
    assert_eq!(names, vec!["src", "top.txt"], "never recurses (§10.1)");
}

#[test]
fn a_range_past_the_end_yields_no_bytes_rather_than_an_error() {
    let fs = FakeFileSystem::new();
    fs.file("/w/a", b"12345");
    assert_eq!(fs.read_range(Path::new("/w/a"), 3, 10).unwrap(), b"45");
    assert_eq!(
        fs.read_range(Path::new("/w/a"), 99, 10).unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn it_can_be_told_to_fail() {
    let fs = FakeFileSystem::new();
    fs.file("/w/a", b"x");
    fs.fail(std::io::ErrorKind::PermissionDenied);
    assert!(
        fs.metadata(Path::new("/w/a")).is_err(),
        "a fake that cannot fail tests only the happy path"
    );
}
