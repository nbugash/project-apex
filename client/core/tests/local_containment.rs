//! §6.4: a local workspace is still a workspace, and path escapes are still bugs.
//!
//! The mirror of `engine/tests/path_containment.rs`. Two enforcements rather than one shared
//! implementation, because Principle VI requires each side to validate independently and a
//! shared one would make "independently" a word rather than a fact.
//!
//! Against a real temp tree, because the symlink case cannot be proven against a fake:
//! resolving a symlink is exactly the filesystem behaviour under test.

use apex_shell::adapters::outbound::local_workspace::LocalWorkspaceProvider;
use apex_shell::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use apex_shell::domain::workspace::{PathError, RelPath, WorkspaceId};
use std::fs;
use std::path::PathBuf;

struct Tree {
    provider: LocalWorkspaceProvider,
    root: PathBuf,
    _dir: tempfile::TempDir,
}

fn tree() -> Tree {
    let dir = tempfile::tempdir().expect("temp");
    let root = dir.path().join("workspace");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), b"fn main() {}").unwrap();
    // Outside the root, and real: the thing an escape would reach.
    fs::create_dir_all(dir.path().join("outside")).unwrap();
    fs::write(dir.path().join("outside/secret"), b"s3cret").unwrap();
    let provider = LocalWorkspaceProvider::open(&root).expect("open");
    Tree {
        provider,
        root,
        _dir: dir,
    }
}

fn ws() -> WorkspaceId {
    WorkspaceId("local".into())
}

#[tokio::test]
async fn a_path_inside_the_root_resolves() {
    let t = tree();
    let chunk = t
        .provider
        .read_file(&ws(), &RelPath::parse("/src/main.rs").unwrap(), None)
        .await
        .expect("must read");
    assert_eq!(chunk.bytes, b"fn main() {}");
}

#[test]
fn dot_dot_never_survives_construction() {
    // The lexical stage, before the filesystem is consulted at all. A provider never sees these
    // because `RelPath` refuses to exist with them in it.
    for raw in [
        "../outside/secret",
        "/src/../../outside/secret",
        "/..",
        "..",
    ] {
        assert_eq!(RelPath::parse(raw), Err(PathError::Traversal), "{raw}");
    }
}

#[tokio::test]
async fn a_symlink_resolving_outside_the_root_is_refused() {
    let t = tree();
    // Lexically innocent: no `..` anywhere, so only canonicalisation catches it (FR-006).
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        t.root.parent().unwrap().join("outside"),
        t.root.join("escape"),
    )
    .unwrap();

    assert_eq!(
        t.provider
            .read_file(&ws(), &RelPath::parse("/escape/secret").unwrap(), None)
            .await
            .unwrap_err(),
        ProviderError::Refused,
        "a symlink out of the root must be refused even though the path reads as contained — \
         §6.4 is explicit that a local workspace is still a workspace"
    );
}

#[tokio::test]
async fn a_refusal_does_not_reveal_whether_the_target_exists() {
    let t = tree();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        t.root.parent().unwrap().join("outside"),
        t.root.join("escape"),
    )
    .unwrap();

    // One escape to a real file, one to a path that is not there. FR-007: a caller must not be
    // able to tell them apart, or refusals become a way to probe the filesystem.
    let exists = t
        .provider
        .stat(&ws(), &RelPath::parse("/escape/secret").unwrap())
        .await
        .unwrap_err();
    let absent = t
        .provider
        .stat(&ws(), &RelPath::parse("/escape/no-such-file").unwrap())
        .await
        .unwrap_err();
    assert_eq!(exists, ProviderError::Refused);
    assert_eq!(absent, exists, "the two refusals must be indistinguishable");
}

#[tokio::test]
async fn a_missing_path_inside_the_root_is_not_found_rather_than_refused() {
    let t = tree();
    assert_eq!(
        t.provider
            .stat(&ws(), &RelPath::parse("/src/absent.rs").unwrap())
            .await
            .unwrap_err(),
        ProviderError::NotFound,
        "inside the root and absent is information the caller is entitled to; conflating it with \
         a refusal would make every miss look like an attack"
    );
}

#[tokio::test]
async fn a_root_that_is_not_a_directory_is_refused_at_construction() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file");
    fs::write(&file, b"x").unwrap();
    assert!(
        LocalWorkspaceProvider::open(&file).is_err(),
        "refused here rather than on the first read, so the failure names the workspace instead \
         of a file inside it"
    );
    assert!(LocalWorkspaceProvider::open(&dir.path().join("absent")).is_err());
}

#[tokio::test]
async fn listing_is_shallow_and_ordered_like_the_remote_one() {
    let t = tree();
    fs::write(t.root.join("README.md"), b"# hi").unwrap();
    fs::create_dir_all(t.root.join("zzz")).unwrap();

    let page = t
        .provider
        .read_directory(&ws(), &RelPath::root(), Default::default())
        .await
        .expect("list");
    let names: Vec<&str> = page.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["src", "zzz", "README.md"],
        "directories first, then byte-wise by name — the same order the wire promises, so a \
         consumer cannot tell the two providers apart by what comes back"
    );
}
