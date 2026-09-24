//! FR-021a through FR-021c: nothing unverified reaches the developer while connected, the wait is
//! visible, and it always ends.
//!
//! The wedged-engine case uses tokio's paused clock, so it completes in microseconds. A test that
//! actually sleeps for the two-second limit is a test nobody runs on every commit — and the limit
//! is the thing under test, so it has to be reachable cheaply.

mod common;

use apex_shell::adapters::outbound::stub_connection::StubConnectionStatusSource;
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::ports::workspace_provider::WorkspaceProvider;
use apex_shell::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use apex_shell::domain::cache::Presentation;
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::workspace::{
    EntryKind, FsEntry, Location, RelPath, Workspace, WorkspaceId,
};
use common::fake_cache::InMemoryCache;
use common::fake_clock::FakeClock;
use common::fake_workspace::FakeWorkspace;
use std::sync::{Arc, Mutex};

struct Harness {
    provider: CachedWorkspace,
    engine: Arc<FakeWorkspace>,
    published: Arc<Mutex<Vec<Presentation>>>,
    ws: WorkspaceId,
}

fn harness() -> Harness {
    let engine = Arc::new(FakeWorkspace::new());
    engine.file("/a.rs", b"contents");

    let cache = Arc::new(InMemoryCache::new());
    let ws = WorkspaceId("w1".into());
    cache
        .register(
            &Workspace {
                id: ws.clone(),
                name: "r".into(),
                location: Location::Remote {
                    host: "h".into(),
                    base: "/b".into(),
                },
                last_opened_at: 0,
            },
            0,
        )
        .unwrap();
    cache
        .put_listing(
            &ws,
            &RelPath::root(),
            &[FsEntry {
                name: "a.rs".into(),
                kind: EntryKind::File,
                size: 8,
                modified: 0,
            }],
        )
        .unwrap();

    let conn = StubConnectionStatusSource::new();
    conn.set(ConnectionState::Connected);

    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let p = published.clone();
        Arc::new(move |s: Presentation| p.lock().unwrap().push(s))
    };

    let provider = CachedWorkspace::new(
        engine.clone(),
        cache,
        Arc::new(FakeClock::at(500)),
        Arc::new(conn),
        sink,
        Limits::default(),
    );
    Harness {
        provider,
        engine,
        published,
        ws,
    }
}

fn path() -> RelPath {
    RelPath::parse("/a.rs").unwrap()
}

/// FR-021c, SC-004b. The engine is reachable but never answers.
#[tokio::test(start_paused = true)]
async fn a_confirmation_that_never_arrives_ends_at_the_limit_and_yields_unverified() {
    let h = harness();
    // Prime the cache while the engine still answers.
    let _ = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("first open");
    h.published.lock().unwrap().clear();

    // Now wedge it: `stat` never returns.
    h.engine.wedge("stat");

    let chunk = h
        .provider
        .read_file(&h.ws, &path(), None)
        .await
        .expect("the wait must end with an outcome, never hang");

    assert_eq!(chunk.bytes, b"contents", "the cached copy is offered");
    assert_eq!(
        *h.published.lock().unwrap(),
        vec![Presentation::Verifying, Presentation::Unverified],
        "the developer sees the wait, then is told the content could not be verified — never an \
         indefinite wait, because a wedged engine is exactly when the cache is most useful"
    );
}

/// FR-021b: published for the **whole** time the confirmation is outstanding, not once at the top.
#[tokio::test(start_paused = true)]
async fn verifying_is_published_before_the_confirmation_is_issued() {
    let h = harness();
    let _ = h.provider.read_file(&h.ws, &path(), None).await.unwrap();
    h.published.lock().unwrap().clear();
    h.engine.calls.lock().unwrap().clear();

    let _ = h.provider.read_file(&h.ws, &path(), None).await.unwrap();

    let published = h.published.lock().unwrap();
    assert_eq!(
        published.first(),
        Some(&Presentation::Verifying),
        "published before the stat, so a slow confirmation is visible rather than presenting as \
         a frozen window"
    );
    assert_eq!(h.engine.call_count("stat"), 1);
}

/// SC-004a: while connected, unverified content reaches the developer zero times.
#[tokio::test(start_paused = true)]
async fn no_bytes_reach_the_caller_before_the_confirmation_resolves() {
    let h = harness();
    let _ = h.provider.read_file(&h.ws, &path(), None).await.unwrap();
    h.published.lock().unwrap().clear();

    let chunk = h.provider.read_file(&h.ws, &path(), None).await.unwrap();

    // The only orderings a caller can observe are Verifying->Current, Verifying->Unverified, or a
    // refetch. In every one, `Verifying` precedes any byte being returned.
    let published = h.published.lock().unwrap();
    assert!(!published.is_empty(), "something must have been published");
    assert_eq!(published[0], Presentation::Verifying);
    assert!(
        published
            .iter()
            .any(|p| matches!(p, Presentation::Current | Presentation::Unverified)),
        "and it must resolve"
    );
    assert_eq!(chunk.bytes, b"contents");
}

/// A cache miss has nothing to verify, so it never publishes `Verifying`.
#[tokio::test(start_paused = true)]
async fn a_cache_miss_fetches_without_claiming_to_verify_anything() {
    let h = harness();
    let _ = h.provider.read_file(&h.ws, &path(), None).await.unwrap();
    assert_eq!(
        *h.published.lock().unwrap(),
        vec![Presentation::Current],
        "there was no cached copy to confirm, so announcing a verification would be a lie about \
         what the system is doing"
    );
}

/// The limit is a value, not a constant buried in the code: a caller that knows its own budget
/// states it (A-DEADLINE).
#[tokio::test(start_paused = true)]
async fn the_confirmation_limit_is_injectable_so_the_budget_is_visible() {
    assert_eq!(
        Limits::default().confirm,
        std::time::Duration::from_secs(2),
        "eight times §1.4's 250 ms uncached budget: tight enough that a wedged engine is not \
         mistaken for a hang, loose enough that ordinary transcontinental latency does not \
         train developers to ignore the unverified marker"
    );
}
