//! US5: an outage is an inconvenience rather than a stop.

mod common;

use apex_shell::adapters::outbound::stub_connection::StubConnectionStatusSource;
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
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
    cache: Arc<InMemoryCache>,
    conn: Arc<StubConnectionStatusSource>,
    published: Arc<Mutex<Vec<Presentation>>>,
    ws: WorkspaceId,
}

fn harness() -> Harness {
    let engine = Arc::new(FakeWorkspace::new());
    engine.file("/src/main.rs", b"cached content");
    engine.file("/src/never.rs", b"never opened");

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
                name: "src".into(),
                kind: EntryKind::Directory,
                size: 0,
                modified: 0,
            }],
        )
        .unwrap();
    let src = RelPath::parse("/src").unwrap();
    cache
        .put_listing(
            &ws,
            &src,
            &[
                FsEntry {
                    name: "main.rs".into(),
                    kind: EntryKind::File,
                    size: 14,
                    modified: 0,
                },
                FsEntry {
                    name: "never.rs".into(),
                    kind: EntryKind::File,
                    size: 12,
                    modified: 0,
                },
            ],
        )
        .unwrap();

    let conn = Arc::new(StubConnectionStatusSource::new());
    conn.set(ConnectionState::Connected);

    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let p = published.clone();
        Arc::new(move |s: Presentation| p.lock().unwrap().push(s))
    };

    let provider = CachedWorkspace::new(
        engine.clone(),
        cache.clone(),
        Arc::new(FakeClock::at(1)),
        conn.clone(),
        sink,
        Limits::default(),
    );
    Harness {
        provider,
        engine,
        cache,
        conn,
        published,
        ws,
    }
}

/// SC-011. The count is what matters: asserting only that the search *succeeded* would pass for
/// an implementation that tried the network, timed out, and fell back.
#[tokio::test]
async fn path_search_offline_attempts_zero_requests() {
    let h = harness();
    h.conn.set(ConnectionState::Disconnected);
    h.engine.calls.lock().unwrap().clear();

    let hits = h.cache.search_paths(&h.ws, "main", 10).expect("search");
    assert!(
        hits.iter().any(|p| p.as_str() == "/src/main.rs"),
        "found: {hits:?}"
    );
    assert_eq!(
        h.engine.total_calls(),
        0,
        "zero requests attempted — counted at the transport double, not inferred from the search \
         having returned something"
    );
}

/// US5.2, FR-032.
#[tokio::test]
async fn a_cached_file_is_served_offline_and_marked_possibly_stale() {
    let h = harness();
    let path = RelPath::parse("/src/main.rs").unwrap();
    let _ = h
        .provider
        .read_file(&h.ws, &path, None)
        .await
        .expect("prime while connected");

    h.conn.set(ConnectionState::Disconnected);
    h.published.lock().unwrap().clear();
    h.engine.calls.lock().unwrap().clear();

    let chunk = h
        .provider
        .read_file(&h.ws, &path, None)
        .await
        .expect("served from the projection");
    assert_eq!(chunk.bytes, b"cached content");
    assert_eq!(
        *h.published.lock().unwrap(),
        vec![Presentation::PossiblyStale],
        "presented as possibly stale rather than as current: nobody could confirm it"
    );
    assert_eq!(h.engine.total_calls(), 0, "and no request was attempted");
}

/// US5.3, SC-012.
#[tokio::test]
async fn an_uncached_file_offline_is_reported_unavailable_rather_than_shown_empty() {
    let h = harness();
    h.conn.set(ConnectionState::Disconnected);
    h.engine.calls.lock().unwrap().clear();

    let path = RelPath::parse("/src/never.rs").unwrap();
    let err = h
        .provider
        .read_file(&h.ws, &path, None)
        .await
        .expect_err("must refuse");
    assert_eq!(err, ProviderError::Offline);
    assert_eq!(
        *h.published.lock().unwrap(),
        vec![Presentation::Unavailable],
        "a stated reason, never an empty document — an empty editor is indistinguishable from a \
         file that is genuinely empty"
    );
    assert_eq!(h.engine.total_calls(), 0);
}

/// The connection is consulted before the cache, so an outage costs nothing and times out never.
#[tokio::test]
async fn going_offline_costs_no_request_and_no_timeout() {
    let h = harness();
    h.conn.set(ConnectionState::Disconnected);
    h.engine.wedge("stat");
    h.engine.wedge("read_file");
    h.engine.calls.lock().unwrap().clear();

    // If the connection were checked *after* a failure, these would hang on the wedged engine.
    let path = RelPath::parse("/src/never.rs").unwrap();
    let _ = h
        .provider
        .read_file(&h.ws, &path, None)
        .await
        .expect_err("refused");
    let _ = h
        .provider
        .read_directory(
            &h.ws,
            &RelPath::parse("/unfetched").unwrap(),
            Default::default(),
        )
        .await
        .expect_err("refused");
    assert_eq!(
        h.engine.total_calls(),
        0,
        "nothing was attempted, so nothing could hang"
    );
}

/// The tree stays browsable offline for what has already been fetched.
#[tokio::test]
async fn the_cached_tree_is_still_navigable_offline() {
    let h = harness();
    h.conn.set(ConnectionState::Disconnected);
    h.engine.calls.lock().unwrap().clear();

    let page = h
        .provider
        .read_directory(&h.ws, &RelPath::parse("/src").unwrap(), Default::default())
        .await
        .expect("served from the projection");
    let names: Vec<&str> = page.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["main.rs", "never.rs"]);
    assert_eq!(h.engine.total_calls(), 0);
}
