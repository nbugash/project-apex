//! §4.7 and Principle VI: the engine canonicalises every path it receives and asserts it is a
//! descendant of the workspace root, independently of anything the client checked.
//!
//! Against a **real** temp-directory tree, because the symlink case cannot be proven against a
//! fake — resolving a symlink is exactly the filesystem behaviour under test.

use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::domain::path::{PathRefusal, ResolvedPath};
use std::fs;
use std::path::PathBuf;

/// A workspace root with a file in it, and a sibling directory outside it.
struct Tree {
    root: PathBuf,
    _dir: tempfile::TempDir,
}

impl Tree {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("workspace");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), b"fn main() {}").unwrap();
        // Outside the root, and real: the thing an escape would reach.
        fs::create_dir_all(dir.path().join("outside")).unwrap();
        fs::write(dir.path().join("outside/secret"), b"s3cret").unwrap();
        Self { root, _dir: dir }
    }

    fn resolve(&self, relative: &str) -> Result<ResolvedPath, PathRefusal> {
        let fs_port = StdFileSystem;
        let root = ResolvedPath::canonical_root(&self.root, &fs_port).expect("root canonicalises");
        ResolvedPath::resolve(&root, relative, &fs_port)
    }
}

#[test]
fn a_path_inside_the_root_resolves() {
    let t = Tree::new();
    let p = t.resolve("/src/main.rs").expect("must resolve");
    assert!(
        p.as_path().ends_with("workspace/src/main.rs"),
        "{:?}",
        p.as_path()
    );
}

#[test]
fn dot_dot_traversal_is_refused() {
    let t = Tree::new();
    for raw in [
        "../outside/secret",
        "/../outside/secret",
        "/src/../../outside/secret",
        "/..",
    ] {
        assert_eq!(
            t.resolve(raw),
            Err(PathRefusal::Refused),
            "must refuse {raw:?}"
        );
    }
}

#[test]
fn a_symlink_resolving_outside_the_root_is_refused() {
    let t = Tree::new();
    // Lexically innocent: no `..` anywhere. Only canonicalisation catches it, which is why the
    // check has a second stage (FR-006).
    let link = t.root.join("escape");
    #[cfg(unix)]
    std::os::unix::fs::symlink(t.root.parent().unwrap().join("outside"), &link).unwrap();
    assert_eq!(
        t.resolve("/escape/secret"),
        Err(PathRefusal::Refused),
        "a symlink out of the root must be refused even though the path reads as contained"
    );
}

#[test]
fn a_refusal_does_not_reveal_whether_the_target_exists() {
    let t = Tree::new();
    // One escape to a real file, one to a path that is not there. FR-007: the caller must not be
    // able to tell them apart, or a refusal becomes a way to probe the host's filesystem.
    let exists = t.resolve("/../outside/secret");
    let absent = t.resolve("/../outside/no-such-file");
    assert_eq!(exists, Err(PathRefusal::Refused));
    assert_eq!(absent, Err(PathRefusal::Refused));
    assert_eq!(exists, absent, "the two refusals must be indistinguishable");
}

#[test]
fn a_missing_path_inside_the_root_is_not_found_rather_than_refused() {
    let t = Tree::new();
    assert_eq!(
        t.resolve("/src/absent.rs"),
        Err(PathRefusal::NotFound),
        "inside the root and absent is information the caller is entitled to; conflating it with \
         a refusal would make every miss look like an attack"
    );
}

#[test]
fn an_unchecked_path_from_a_client_is_still_refused() {
    let t = Tree::new();
    // FR-008: the engine's safety must not depend on the client having validated anything. This
    // is the raw string a stale or hostile client would send, with no RelPath in sight.
    assert_eq!(
        t.resolve("/src/../../outside/secret"),
        Err(PathRefusal::Refused)
    );
    assert_eq!(t.resolve("\\..\\outside"), Err(PathRefusal::Refused));
}

#[test]
fn a_nul_byte_is_refused_before_it_reaches_the_filesystem() {
    let t = Tree::new();
    assert_eq!(t.resolve("/src/main\0.rs"), Err(PathRefusal::Refused));
}
