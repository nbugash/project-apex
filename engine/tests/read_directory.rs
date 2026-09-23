//! `workspace/readDirectory` (§4.8, §10.1): shallow, ordered, paged.
mod common;

use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::{self, InMemoryRoots};
use apex_protocol::wire::EntryKind;
use common::FakeFileSystem;
use std::sync::Arc;

fn tree() -> (Arc<FakeFileSystem>, InMemoryRoots) {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    fs.file("/w/README.md", b"z");
    fs.file("/w/src/main.rs", b"x");
    fs.file("/w/src/deep/other.rs", b"y");
    fs.dir("/w/zzz");
    let roots = InMemoryRoots::new(fs.clone());
    roots.register("w1", "/w").unwrap();
    (fs, roots)
}

fn page(
    fs: &FakeFileSystem,
    roots: &InMemoryRoots,
    rel: &str,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> (Vec<String>, Option<String>) {
    let path = workspace::resolve_request(roots, fs, "w1", rel).expect("resolves");
    let (items, next) = workspace::read_directory(fs, &path, cursor, limit).expect("lists");
    (items.iter().map(|e| e.name.clone()).collect(), next)
}

#[test]
fn a_listing_is_shallow_and_never_recurses() {
    let (fs, roots) = tree();
    let (names, _) = page(&fs, &roots, "/", None, None);
    assert_eq!(names, vec!["src", "zzz", "README.md"]);
    assert!(
        !names.contains(&"other.rs".to_string()),
        "§10.1: immediate children only"
    );
}

#[test]
fn the_order_is_directories_first_then_bytewise_by_name() {
    let (fs, roots) = tree();
    let (names, _) = page(&fs, &roots, "/", None, None);
    assert_eq!(
        names,
        vec!["src", "zzz", "README.md"],
        "uppercase sorts after lowercase here because the comparison is byte-wise on UTF-8, and \
         that is contractual: the pagination cursor is the last name returned, so both ends must \
         derive the same order or a page boundary lands in a different place"
    );
}

#[test]
fn directories_are_typed_as_directories() {
    let (fs, roots) = tree();
    let path = workspace::resolve_request(&roots, fs.as_ref(), "w1", "/").unwrap();
    let (items, _) = workspace::read_directory(fs.as_ref(), &path, None, None).unwrap();
    let src = items.iter().find(|e| e.name == "src").expect("src");
    assert_eq!(src.kind, EntryKind::Directory);
    let readme = items
        .iter()
        .find(|e| e.name == "README.md")
        .expect("readme");
    assert_eq!(readme.kind, EntryKind::File);
}

#[test]
fn paging_walks_every_entry_exactly_once() {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    for n in ["a", "b", "c", "d", "e"] {
        fs.file(&format!("/w/{n}.rs"), b"x");
    }
    let roots = InMemoryRoots::new(fs.clone());
    roots.register("w1", "/w").unwrap();

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let (names, next) = page(&fs, &roots, "/", cursor.as_deref(), Some(2));
        seen.extend(names);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(
        seen,
        vec!["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"],
        "no gaps and no repeats — which offset paging cannot promise once a directory changes \
         between pages"
    );
}

#[test]
fn the_limit_is_capped_at_the_protocol_maximum() {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    for n in 0..5 {
        fs.file(&format!("/w/f{n}.rs"), b"x");
    }
    let roots = InMemoryRoots::new(fs.clone());
    roots.register("w1", "/w").unwrap();
    let (names, next) = page(&fs, &roots, "/", None, Some(99_999));
    assert_eq!(names.len(), 5, "a caller cannot ask for more than the cap");
    assert_eq!(next, None);
}

/// The bug a name-only cursor would have: directories and files interleave in the ordering, so
/// a cursor that compares names alone skips every file whose name sorts before the last directory.
#[test]
fn paging_a_directory_that_mixes_files_and_directories_loses_nothing() {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    fs.dir("/w/a");
    fs.dir("/w/z");
    fs.file("/w/b.rs", b"x");
    let roots = InMemoryRoots::new(fs.clone());
    roots.register("w1", "/w").unwrap();

    // The listing order is a, z (directories) then b.rs. A name-only cursor after "z" would find
    // no later *name* and drop b.rs on the floor.
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let (names, next) = page(&fs, &roots, "/", cursor.as_deref(), Some(2));
        seen.extend(names);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(
        seen,
        vec!["a", "z", "b.rs"],
        "every entry exactly once across a type boundary — the case an all-files fixture cannot \
         detect"
    );
}

#[test]
fn a_cursor_naming_a_vanished_entry_resumes_where_it_would_have_sorted() {
    let (fs, roots) = tree();
    let path = workspace::resolve_request(&roots, fs.as_ref(), "w1", "/").unwrap();
    let (items, _) = workspace::read_directory(fs.as_ref(), &path, None, None).unwrap();
    let after_src = workspace::page_cursor(items.iter().find(|e| e.name == "src").unwrap());

    let (names, _) = page(&fs, &roots, "/", Some(&after_src), None);
    assert_eq!(names, vec!["zzz", "README.md"]);

    // A token for an entry that is gone still positions the page rather than failing.
    let ghost = "0\u{1f}deleted-since".to_string();
    let (names, _) = page(&fs, &roots, "/", Some(&ghost), None);
    assert_eq!(
        names,
        vec!["src", "zzz", "README.md"],
        "resuming after a vanished directory lands where that name would have sorted"
    );
}

#[test]
fn a_cursor_past_the_last_entry_returns_nothing_without_a_next_cursor() {
    let (fs, roots) = tree();
    let (names, next) = page(&fs, &roots, "/", Some("9"), None);
    assert!(names.is_empty());
    assert_eq!(next, None);
}
