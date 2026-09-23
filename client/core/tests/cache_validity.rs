//! US2, and the rule §5.3 says this project has already got wrong once.
//!
//! Cache validity is a hash comparison and nothing else. These tests are written before the
//! behaviour they police, per Principle VII, which names cache validity as a case where a wrong
//! answer is expensive.

mod common;

use apex_shell::adapters::outbound::stub_connection::StubConnectionStatusSource;
use apex_shell::application::ports::workspace_cache::{StoreOutcome, WorkspaceCache};
use apex_shell::application::ports::workspace_provider::WorkspaceProvider;
use apex_shell::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use apex_shell::domain::cache::Presentation;
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::workspace::{
    ByteRange, EntryKind, FsEntry, Location, RelPath, Sha256, Workspace, WorkspaceId,
};
use common::fake_cache::InMemoryCache;
use common::fake_clock::FakeClock;
use common::fake_workspace::FakeWorkspace;
use std::sync::{Arc, Mutex};

struct Harness {
    provider: CachedWorkspace,
    engine: Arc<FakeWorkspace>,
    cache: Arc<InMemoryCache>,
    published: Arc<Mutex<Vec<Presentation>>>,
    ws: WorkspaceId,
}

fn harness(connected: bool) -> Harness {
    let engine = Arc::new(FakeWorkspace::new());
    engine.file("/src/main.rs", b"fn main() {}");

    let cache = Arc::new(InMemoryCache::new());
    let ws = WorkspaceId("w1".into());
    cache
        .register(
            &Workspace {
                id: ws.clone(),
                name: "repo".into(),
                location: Location::Remote {
                    host: "h".into(),
                    base: "/b".into(),
                },
                last_opened_at: 0,
            },
            0,
        )
        .unwrap();
    let src = RelPath::parse("/src").unwrap();
    cache
        .put_listing(
            &ws,
            &RelPath::root(),
            &[FsEntry {
                name: "src".into(),
                kind: EntryKind::Directory,
                size: 0,
                modified: 0,
            }],
        )
        .unwrap();
    cache
        .put_listing(
            &ws,
            &src,
            &[FsEntry {
                name: "main.rs".into(),
                kind: EntryKind::File,
                size: 12,
                modified: 0,
            }],
        )
        .unwrap();

    let conn = StubConnectionStatusSource::new();
    conn.set(if connected {
        ConnectionState::Connected
    } else {
        ConnectionState::Disconnected
    });

    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let p = published.clone();
        Arc::new(move |s: Presentation| p.lock().unwrap().push(s))
    };

    let provider = CachedWorkspace::new(
        engine.clone(),
        cache.clone(),
        Arc::new(FakeClock::at(1_000)),
        Arc::new(conn),
        sink,
        Limits::default(),
    );
    Harness {
        provider,
        engine,
        cache,
        published,
        ws,
    }
}

fn path() -> RelPath {
    RelPath::parse("/src/main.rs").unwrap()
}

/// Put the current engine content into the cache, as a first open would.
async fn prime(h: &Harness) {
    let _ = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("first open fetches");
    h.engine
        .content_bytes
        .store(0, std::sync::atomic::Ordering::SeqCst);
    h.engine.calls.lock().unwrap().clear();
}

/// US2.1, SC-003.
#[tokio::test]
async fn a_matching_hash_serves_from_cache_and_transfers_zero_content_bytes() {
    let h = harness(true);
    prime(&h).await;

    let chunk = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("second open");
    assert_eq!(chunk.bytes, b"fn main() {}");
    assert_eq!(
        h.engine.bytes_transferred(),
        0,
        "a cached file whose hash matches must transfer no content at all (SC-003)"
    );
    assert_eq!(h.engine.call_count("read_file"), 0, "no read was issued");
    assert_eq!(h.engine.call_count("stat"), 1, "exactly one confirmation");
}

/// US2.2, SC-004.
#[tokio::test]
async fn a_changed_hash_refetches_and_replaces_the_cached_copy() {
    let h = harness(true);
    prime(&h).await;

    h.engine.rewrite("/src/main.rs", b"fn main() { changed }");

    let chunk = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("open");
    assert_eq!(
        chunk.bytes, b"fn main() { changed }",
        "content that changed remotely is never served from the cache (SC-004)"
    );
    let again = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("open again");
    assert_eq!(
        again.bytes, b"fn main() { changed }",
        "and the cache now holds the new content"
    );
}

/// US2.5, SC-005. The scenario that fails if anyone wires git status into validity.
#[tokio::test]
async fn a_file_modified_in_git_but_unchanged_in_content_is_still_served_from_cache() {
    let h = harness(true);
    prime(&h).await;

    // There is deliberately no way to express "this file is MODIFIED in git" to the cache: the
    // `Validity` type has one constructor and it takes two hashes. This test asserts the
    // consequence — the content is unchanged, so it is served — and the *absence* of a git input
    // is what guarantees it stays true.
    let chunk = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("open");
    assert_eq!(chunk.bytes, b"fn main() {}");
    assert_eq!(
        h.engine.bytes_transferred(),
        0,
        "invalidating on git status would discard cached content for exactly the files being \
         worked on, forcing a refetch on every save (§5.3, FR-020)"
    );
}

/// US2.6, FR-023.
#[tokio::test]
async fn a_ranged_read_returns_the_beginning_without_the_whole() {
    let h = harness(true);
    prime(&h).await;

    let head = h
        .provider
        .read_file(
            &h.ws,
            &path(),
            Some(ByteRange {
                offset: 0,
                length: 2,
            }),
        )
        .await
        .expect("ranged read");
    assert_eq!(head.bytes, b"fn");
    assert_eq!(
        head.total_size, 12,
        "the whole file's size is known from the first range"
    );
    assert_eq!(
        head.sha256,
        Sha256::of(b"fn main() {}"),
        "the digest describes the whole file, so a caller assembling ranges can tell the file \
         moved underneath the read (FR-021)"
    );
}

/// FR-034: caching is an optimisation.
#[tokio::test]
async fn a_failing_cache_write_does_not_fail_the_read() {
    let h = harness(true);
    h.cache.fail_writes("disk full");

    let chunk = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("the read must succeed");
    assert_eq!(
        chunk.bytes, b"fn main() {}",
        "a full disk changes what is stored, never what is returned"
    );
    assert!(!h.cache.is_cached(&h.ws, &path()), "and nothing was stored");
}

/// The published sequence, which is what the interface renders.
#[tokio::test]
async fn a_cache_hit_publishes_verifying_before_current() {
    let h = harness(true);
    prime(&h).await;
    h.published.lock().unwrap().clear();

    let _ = h.provider.read_file(&h.ws, &path(), None).await.unwrap();
    assert_eq!(
        *h.published.lock().unwrap(),
        vec![Presentation::Verifying, Presentation::Current],
        "verifying is published before the confirmation is issued and resolves to current only \
         after it returns (FR-021a, FR-021b)"
    );
}

/// FR-028: retention measures use, not age.
#[tokio::test]
async fn every_cache_hit_records_its_access() {
    let h = harness(true);
    prime(&h).await;
    let file_id = h.cache.file_id(&h.ws, &path()).unwrap().unwrap();
    assert_eq!(h.cache.touch(&file_id, 0), StoreOutcome::Stored);

    let _ = h.provider.read_file(&h.ws, &path(), None).await.unwrap();
    let entry = h.cache.lookup(&h.ws, &path()).unwrap().unwrap();
    assert_eq!(
        entry.last_accessed_at, 1_000,
        "the hit must stamp the clock's now, or a file read daily would still be evicted as \
         though it had not been touched in a fortnight"
    );
}
