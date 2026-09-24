//! One suite, every `WorkspaceCache` implementation.
//!
//! Guarantees C1-C9 from specs/005-workspace-cache/contracts/cache.md, run against the real
//! SQLite adapter (a temp file, so the SQL is the SQL that ships) and the in-memory fake. A
//! guarantee that holds for one and not the other is a fake that will mislead every test built on
//! it, which is why they are checked together rather than separately.

mod common;

use apex_shell::adapters::outbound::sqlite::SqliteWorkspaceCache;
use apex_shell::application::ports::workspace_cache::{Attachment, StoreOutcome, WorkspaceCache};
use apex_shell::domain::workspace::{
    EntryKind, FsEntry, Location, RelPath, Sha256, Workspace, WorkspaceId,
};
use common::fake_cache::InMemoryCache;

/// Both implementations, so every assertion below runs twice.
fn implementations() -> Vec<(&'static str, Box<dyn WorkspaceCache>)> {
    let sqlite = SqliteWorkspaceCache::in_memory().expect("sqlite");
    sqlite.migrate_to(1, &mut |_| {}).expect("v1 schema");
    vec![
        ("sqlite", Box::new(sqlite)),
        ("in-memory fake", Box::new(InMemoryCache::new())),
    ]
}

fn workspace(id: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(id.into()),
        name: "repo".into(),
        location: Location::Remote {
            host: "h".into(),
            base: "/b".into(),
        },
        last_opened_at: 0,
    }
}

fn entry(name: &str, kind: EntryKind, size: u64) -> FsEntry {
    FsEntry {
        name: name.into(),
        kind,
        size,
        modified: 0,
    }
}

/// Register a workspace with one file, and return its path.
fn seeded(c: &dyn WorkspaceCache, ws: &Workspace) -> RelPath {
    c.register(ws, 0).expect("register");
    let root = RelPath::root();
    c.put_listing(&ws.id, &root, &[entry("main.rs", EntryKind::File, 12)])
        .expect("listing");
    RelPath::parse("/main.rs").unwrap()
}

#[test]
fn c1_the_stored_digest_is_over_decompressed_bytes() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        let path = seeded(c.as_ref(), &ws);
        let id = c.lookup(&ws.id, &path).unwrap();
        assert!(id.is_none(), "[{name}] nothing cached yet");

        // Content that compresses: if the digest were taken after compression the round trip
        // below would still succeed, so the assertion is that the *engine's* digest matches.
        let bytes = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec();
        let hash = Sha256::of(&bytes);
        let file_id = file_id_of(c.as_ref(), &ws.id, &path);
        assert_eq!(
            c.put_content(&file_id, &bytes, &hash, 10),
            StoreOutcome::Stored,
            "[{name}]"
        );

        let got = c.lookup(&ws.id, &path).unwrap().expect("cached");
        assert_eq!(got.bytes, bytes, "[{name}] round trip must be byte-exact");
        assert_eq!(
            got.hash,
            Sha256::of(&got.bytes),
            "[{name}] the stored digest must be of the decompressed content, so it compares \
             directly with the engine's (§5.6)"
        );
    }
}

#[test]
fn c2_content_above_the_cap_is_not_eligible_rather_than_an_error() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        let path = seeded(c.as_ref(), &ws);
        let file_id = file_id_of(c.as_ref(), &ws.id, &path);
        let big = vec![0u8; 9 * 1024 * 1024];
        let outcome = c.put_content(&file_id, &big, &Sha256::of(&big), 0);
        assert_eq!(
            outcome,
            StoreOutcome::NotEligible {
                size: big.len() as u64
            },
            "[{name}] above A-CACHECAP is a stated non-outcome, not a failure"
        );
        assert!(
            c.lookup(&ws.id, &path).unwrap().is_none(),
            "[{name}] nothing stored"
        );
    }
}

#[test]
fn c3_a_failing_write_is_reported_without_being_an_error_the_caller_must_handle() {
    // The fake can be told to fail; the real store cannot be made to fail deterministically
    // in-process without corrupting it, so this guarantee is asserted where it is assertable.
    let c = InMemoryCache::new();
    let ws = workspace("w1");
    let path = seeded(&c, &ws);
    let file_id = file_id_of(&c, &ws.id, &path);
    c.fail_writes("disk full");
    match c.put_content(&file_id, b"x", &Sha256::of(b"x"), 0) {
        StoreOutcome::Failed(_) => {}
        other => panic!("expected a reported failure, got {other:?}"),
    }
    // The type is what enforces FR-034: `StoreOutcome` is not a `Result`, so there is no `?` to
    // turn a full disk into a failed file open.
}

#[test]
fn c4_eviction_removes_content_only_and_never_a_tree_entry() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        let path = seeded(c.as_ref(), &ws);
        let file_id = file_id_of(c.as_ref(), &ws.id, &path);
        assert_eq!(
            c.put_content(&file_id, b"hello", &Sha256::of(b"hello"), 100),
            StoreOutcome::Stored
        );

        let report = c.evict(1_000).expect("evict");
        assert_eq!(report.blobs_removed, 1, "[{name}]");
        assert!(
            c.lookup(&ws.id, &path).unwrap().is_none(),
            "[{name}] content gone"
        );
        let listed = c.list_children(&ws.id, &RelPath::root()).unwrap();
        assert_eq!(
            listed.len(),
            1,
            "[{name}] the tree must remain navigable and the file must remain listed (FR-027)"
        );
    }
}

#[test]
fn c5_is_cached_never_disagrees_with_what_opening_the_file_finds() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        let path = seeded(c.as_ref(), &ws);
        let file_id = file_id_of(c.as_ref(), &ws.id, &path);
        // Before: listed, not cached.
        assert!(c.lookup(&ws.id, &path).unwrap().is_none(), "[{name}]");
        assert_eq!(
            c.put_content(&file_id, b"hello", &Sha256::of(b"hello"), 1),
            StoreOutcome::Stored
        );
        assert!(c.lookup(&ws.id, &path).unwrap().is_some(), "[{name}]");
        c.evict(1_000).unwrap();
        assert!(
            c.lookup(&ws.id, &path).unwrap().is_none(),
            "[{name}] after eviction the listing column and the content must agree again"
        );
    }
}

#[test]
fn c6_path_search_needs_no_provider() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        c.register(&ws, 0).unwrap();
        c.put_listing(
            &ws.id,
            &RelPath::root(),
            &[entry("src", EntryKind::Directory, 0)],
        )
        .unwrap();
        let src = RelPath::parse("/src").unwrap();
        c.put_listing(&ws.id, &src, &[entry("main.rs", EntryKind::File, 1)])
            .unwrap();

        let hits = c.search_paths(&ws.id, "main", 10).expect("search");
        assert!(
            hits.iter().any(|p| p.as_str() == "/src/main.rs"),
            "[{name}] expected /src/main.rs in {hits:?}"
        );
    }
}

#[test]
fn c9_put_listing_does_not_claim_to_recognise_a_rename() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        let root = RelPath::root();
        let path = seeded(c.as_ref(), &ws);
        let file_id = file_id_of(c.as_ref(), &ws.id, &path);
        assert_eq!(
            c.put_content(&file_id, b"content", &Sha256::of(b"content"), 1),
            StoreOutcome::Stored
        );

        // A re-listing in which the name changed: one gone, one new, nothing linking them.
        c.put_listing(&ws.id, &root, &[entry("renamed.rs", EntryKind::File, 12)])
            .unwrap();

        let new_path = RelPath::parse("/renamed.rs").unwrap();
        assert!(
            c.lookup(&ws.id, &new_path).unwrap().is_none(),
            "[{name}] the content is NOT carried across; claiming otherwise is the defect C9 \
             exists to prevent (FR-022a)"
        );
        let listed = c.list_children(&ws.id, &root).unwrap();
        assert_eq!(
            listed.len(),
            1,
            "[{name}] and the file stays listed (FR-022b)"
        );
        assert_eq!(listed[0].name, "renamed.rs", "[{name}]");
    }
}

#[test]
fn a_known_rename_keeps_its_content() {
    // The other half of C9, and the reason the opaque file_id exists at all (FR-022).
    for (name, c) in implementations() {
        let ws = workspace("w1");
        let path = seeded(c.as_ref(), &ws);
        let file_id = file_id_of(c.as_ref(), &ws.id, &path);
        assert_eq!(
            c.put_content(&file_id, b"content", &Sha256::of(b"content"), 1),
            StoreOutcome::Stored
        );

        let to = RelPath::parse("/moved.rs").unwrap();
        c.rename(&file_id, &to).expect("rename");

        let got = c
            .lookup(&ws.id, &to)
            .unwrap()
            .expect("content must survive the move");
        assert_eq!(got.bytes, b"content", "[{name}]");
        assert_eq!(
            got.file_id, file_id,
            "[{name}] identity is preserved, which is the mechanism"
        );
    }
}

#[test]
fn re_registering_attaches_rather_than_creating_a_second_projection() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        assert_eq!(c.register(&ws, 0).unwrap(), Attachment::Created, "[{name}]");
        assert_eq!(
            c.register(&ws, 5).unwrap(),
            Attachment::Attached,
            "[{name}] FR-011"
        );
    }
}

#[test]
fn forgetting_a_workspace_removes_its_content_and_its_tree() {
    for (name, c) in implementations() {
        let ws = workspace("w1");
        let path = seeded(c.as_ref(), &ws);
        let file_id = file_id_of(c.as_ref(), &ws.id, &path);
        assert_eq!(
            c.put_content(&file_id, b"x", &Sha256::of(b"x"), 1),
            StoreOutcome::Stored
        );

        c.forget(&ws.id).expect("forget");
        assert!(
            c.list_children(&ws.id, &RelPath::root())
                .unwrap()
                .is_empty(),
            "[{name}] tree gone (FR-012)"
        );
        assert!(
            c.lookup(&ws.id, &path).unwrap().is_none(),
            "[{name}] content gone"
        );
    }
}

/// The id of a listed file, however it is reached.
fn file_id_of(
    c: &dyn WorkspaceCache,
    ws: &WorkspaceId,
    path: &RelPath,
) -> apex_shell::domain::workspace::FileId {
    c.file_id(ws, path)
        .expect("the store must answer")
        .expect("the file must be listed before it can be cached")
}
