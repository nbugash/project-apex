//! One suite, every `WorkspaceProvider` implementation.
//!
//! P1-P6 from specs/005-workspace-cache/contracts/provider.md. This is the mechanism by which
//! §6.1's "the UI never learns which is active" becomes a test rather than an intention: the day
//! it fails against a new implementation is the day the abstraction stopped holding.
//!
//! Adding F015's local provider means adding one line to `implementations()`.

mod common;

use apex_shell::adapters::outbound::local_workspace::LocalWorkspaceProvider;
use apex_shell::application::ports::workspace_provider::{Owner, ProviderError, WorkspaceProvider};
use apex_shell::domain::workspace::{
    ByteRange, EntryKind, PageRequest, RelPath, Sha256, WorkspaceId,
};
use common::fake_workspace::FakeWorkspace;
use std::sync::Arc;

/// Every implementation, seeded identically.
///
/// The `TempDir` is returned alongside, because dropping it would delete the tree the local
/// provider is reading — a guard that looks unused and is the whole reason the local cases pass.
/// A named provider, ready to run the suite against.
type Subject = (&'static str, Arc<dyn WorkspaceProvider>);

fn implementations() -> (Vec<Subject>, tempfile::TempDir) {
    let fake = FakeWorkspace::new();
    fake.file("/src/main.rs", b"fn main() {}");
    fake.file("/README.md", b"# hi");
    fake.dir("/empty");

    // The same tree on a real filesystem. One suite, two implementations, identical content:
    // that is what makes "the UI never learns which is active" (§6.1, FR-001) a test rather
    // than an intention, and adding F015's consumer later changes nothing here.
    let dir = tempfile::tempdir().expect("temp");
    let root = dir.path().join("workspace");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    std::fs::write(root.join("src/main.rs"), b"fn main() {}").unwrap();
    std::fs::write(root.join("README.md"), b"# hi").unwrap();
    let local = LocalWorkspaceProvider::open(&root).expect("open local root");

    (
        vec![
            ("fake", Arc::new(fake) as Arc<dyn WorkspaceProvider>),
            ("local", Arc::new(local) as Arc<dyn WorkspaceProvider>),
        ],
        dir,
    )
}

fn ws() -> WorkspaceId {
    WorkspaceId("w1".into())
}

#[tokio::test]
async fn p2_read_file_returns_bytes_and_never_transcodes() {
    let (impls, _tree) = implementations();
    for (name, p) in impls {
        let path = RelPath::parse("/src/main.rs").unwrap();
        let chunk = p
            .read_file(&ws(), &path, None)
            .await
            .expect("[{name}] read");
        assert_eq!(chunk.bytes, b"fn main() {}", "[{name}] byte-exact");
        assert_eq!(
            chunk.sha256,
            Sha256::of(b"fn main() {}"),
            "[{name}] the digest is of the WHOLE file, never of the returned range (FR-021)"
        );
    }
}

#[tokio::test]
async fn p2_a_ranged_read_still_carries_the_whole_files_digest() {
    let (impls, _tree) = implementations();
    for (name, p) in impls {
        let path = RelPath::parse("/src/main.rs").unwrap();
        let head = p
            .read_file(
                &ws(),
                &path,
                Some(ByteRange {
                    offset: 0,
                    length: 2,
                }),
            )
            .await
            .expect("read");
        assert_eq!(head.bytes, b"fn", "[{name}]");
        assert_eq!(head.total_size, 12, "[{name}] the whole file's size");
        assert_eq!(
            head.sha256,
            Sha256::of(b"fn main() {}"),
            "[{name}] a caller assembling ranges compares this across them; if it described the \
             range it would change every time and detect nothing"
        );
    }
}

#[tokio::test]
async fn p3_a_missing_path_is_not_found() {
    let (impls, _tree) = implementations();
    for (name, p) in impls {
        let path = RelPath::parse("/nope.rs").unwrap();
        assert_eq!(
            p.stat(&ws(), &path).await.unwrap_err(),
            ProviderError::NotFound,
            "[{name}]"
        );
    }
}

#[tokio::test]
async fn p4_unimplemented_methods_refuse_by_name_and_have_no_side_effect() {
    let (impls, _tree) = implementations();
    for (name, p) in impls {
        let path = RelPath::parse("/src/main.rs").unwrap();
        let before = p.read_file(&ws(), &path, None).await.unwrap().bytes;

        let cases: Vec<(&str, ProviderError)> = vec![
            (
                "write_file",
                p.write_file(&ws(), &path, b"x", &Sha256::of(b""))
                    .await
                    .unwrap_err(),
            ),
            (
                "create_file",
                p.create_file(&ws(), &path).await.unwrap_err(),
            ),
            (
                "create_directory",
                p.create_directory(&ws(), &path).await.unwrap_err(),
            ),
            ("rename", p.rename(&ws(), &path, &path).await.unwrap_err()),
            ("delete", p.delete(&ws(), &path, false).await.unwrap_err()),
            ("search", p.search(&ws(), "q").await.unwrap_err()),
            ("watch", p.watch(&ws(), &path).await.unwrap_err()),
        ];
        for (method, err) in cases {
            match err {
                ProviderError::Unsupported { owner } => {
                    assert!(
                        matches!(
                            owner,
                            Owner::F004FileWatch | Owner::F006Editor | Owner::F013Search
                        ),
                        "[{name}] {method} must name its owner so a log reads as a schedule \
                         rather than a bug"
                    );
                }
                other => panic!("[{name}] {method} must refuse explicitly, got {other:?}"),
            }
        }

        let after = p.read_file(&ws(), &path, None).await.unwrap().bytes;
        assert_eq!(
            before, after,
            "[{name}] a refused method must have no side effect. A write_file that returned Ok \
             without writing would be discovered by F006, weeks later, as a bug in F006."
        );
    }
}

#[tokio::test]
async fn p6_errors_are_typed_rather_than_stringly() {
    let (impls, _tree) = implementations();
    for (name, p) in impls {
        // A caller distinguishes these without parsing a message. The test is that the variants
        // are distinct values, not that a message contains a word.
        let missing = p
            .stat(&ws(), &RelPath::parse("/nope").unwrap())
            .await
            .unwrap_err();
        let unsupported = p.watch(&ws(), &RelPath::root()).await.unwrap_err();
        assert_ne!(missing, unsupported, "[{name}]");
        assert_eq!(missing, ProviderError::NotFound, "[{name}]");
    }
}

#[tokio::test]
async fn r1_one_provider_call_issues_at_most_one_request() {
    // Counted at the fake, not parsed out of a log: a log-shape change would silently pass a
    // test that greps. SC-002 is only measurable if this holds — a provider that fanned out
    // would make the listing count stop equalling the folders expanded.
    //
    // A concrete handle is kept beside the trait object, because a count cannot be reached
    // through `dyn WorkspaceProvider` and an assertion that cannot see the count is not an
    // assertion.
    let fake = Arc::new(FakeWorkspace::new());
    fake.file("/a/b.rs", b"x");
    let p: Arc<dyn WorkspaceProvider> = fake.clone();

    assert_eq!(
        fake.total_calls(),
        0,
        "nothing issued before the first call"
    );
    let _ = p
        .read_directory(&ws(), &RelPath::root(), PageRequest::default())
        .await
        .unwrap();
    assert_eq!(
        fake.total_calls(),
        1,
        "one provider call, one request — no fan-out"
    );

    let _ = p
        .stat(&ws(), &RelPath::parse("/a/b.rs").unwrap())
        .await
        .unwrap();
    assert_eq!(fake.total_calls(), 2);

    let _ = p
        .read_file(&ws(), &RelPath::parse("/a/b.rs").unwrap(), None)
        .await
        .unwrap();
    assert_eq!(
        fake.total_calls(),
        3,
        "still one-for-one: no retry, no prefetch"
    );
}

#[tokio::test]
async fn read_directory_is_shallow_and_ordered() {
    let fake = FakeWorkspace::new();
    fake.file("/src/main.rs", b"x");
    fake.file("/src/deep/other.rs", b"y");
    fake.file("/README.md", b"z");
    fake.dir("/zzz");

    let page = fake
        .read_directory(&ws(), &RelPath::root(), PageRequest::default())
        .await
        .unwrap();
    let names: Vec<&str> = page.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["src", "zzz", "README.md"],
        "immediate children only (§10.1), directories first then byte-wise by name — uppercase \
         before lowercase, which is what makes the cursor reproducible on both ends"
    );
    assert!(
        page.items.iter().all(|e| e.name != "other.rs"),
        "never recurses"
    );
    assert_eq!(page.next_cursor, None, "one page holds all three");
    assert_eq!(
        fake.call_count("read_directory"),
        1,
        "exactly one request (R1)"
    );
}

#[tokio::test]
async fn a_page_boundary_yields_a_cursor_that_resumes_without_gaps_or_repeats() {
    let fake = FakeWorkspace::new();
    for n in ["a", "b", "c", "d"] {
        fake.file(&format!("/{n}.rs"), b"x");
    }
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = fake
            .read_directory(&ws(), &RelPath::root(), PageRequest { cursor, limit: 2 })
            .await
            .unwrap();
        seen.extend(page.items.iter().map(|e| e.name.clone()));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(
        seen,
        vec!["a.rs", "b.rs", "c.rs", "d.rs"],
        "every entry exactly once: no gaps, no repeats"
    );
}

#[tokio::test]
async fn a_range_past_the_end_yields_no_bytes_rather_than_an_error() {
    let fake = FakeWorkspace::new();
    fake.file("/a.rs", b"12345");
    let chunk = fake
        .read_file(
            &ws(),
            &RelPath::parse("/a.rs").unwrap(),
            Some(ByteRange {
                offset: 99,
                length: 10,
            }),
        )
        .await
        .expect("must not error");
    assert!(chunk.bytes.is_empty());
    assert_eq!(
        chunk.total_size, 5,
        "which is what lets a caller scroll toward the end without \
                                     racing the file's size"
    );
}

#[tokio::test]
async fn stat_omits_a_digest_for_a_directory() {
    let fake = FakeWorkspace::new();
    fake.dir("/src");
    let meta = fake
        .stat(&ws(), &RelPath::parse("/src").unwrap())
        .await
        .unwrap();
    assert_eq!(meta.kind, EntryKind::Directory);
    assert_eq!(
        meta.sha256, None,
        "there is nothing to hash and no caller that needs it"
    );
}
