//! US1: the cost of browsing is proportional to what the developer opened, not to the repository.
//!
//! Every count is taken at the transport double. A test that grepped a log would pass the day
//! the logging changed shape, which is how a count stops meaning anything.

mod common;

use apex_shell::adapters::outbound::stub_connection::StubConnectionStatusSource;
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::ports::workspace_provider::WorkspaceProvider;
use apex_shell::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use apex_shell::domain::cache::Presentation;
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::workspace::{Location, PageRequest, RelPath, Workspace, WorkspaceId};
use common::fake_cache::InMemoryCache;
use common::fake_clock::FakeClock;
use common::fake_workspace::FakeWorkspace;
use std::sync::Arc;

struct Harness {
    provider: CachedWorkspace,
    engine: Arc<FakeWorkspace>,
    conn: Arc<StubConnectionStatusSource>,
    ws: WorkspaceId,
}

/// A deep, wide tree: `folders` top-level directories, each with `per` files.
fn harness(folders: usize, per: usize) -> Harness {
    let engine = Arc::new(FakeWorkspace::new());
    for f in 0..folders {
        for n in 0..per {
            engine.file(&format!("/dir{f:05}/file{n:05}.rs"), b"x");
        }
    }

    let cache = Arc::new(InMemoryCache::new());
    let ws = WorkspaceId("w1".into());
    cache
        .register(
            &Workspace {
                id: ws.clone(),
                name: "big".into(),
                location: Location::Remote {
                    host: "h".into(),
                    base: "/b".into(),
                },
                last_opened_at: 0,
            },
            0,
        )
        .unwrap();

    let conn = Arc::new(StubConnectionStatusSource::new());
    conn.set(ConnectionState::Connected);

    let provider = CachedWorkspace::new(
        engine.clone(),
        cache,
        Arc::new(FakeClock::at(0)),
        conn.clone(),
        Arc::new(|_: Presentation| {}),
        Limits::default(),
    );
    Harness {
        provider,
        engine,
        conn,
        ws,
    }
}

/// US1.1, SC-001.
#[tokio::test]
async fn opening_a_workspace_fetches_exactly_one_listing() {
    let h = harness(500, 20);
    let _ = h
        .provider
        .read_directory(&h.ws, &RelPath::root(), PageRequest::default())
        .await
        .expect("root listing");
    assert_eq!(
        h.engine.call_count("read_directory"),
        1,
        "opening a workspace of any size fetches exactly one directory listing (§10.1)"
    );
}

/// US1.2.
#[tokio::test]
async fn a_folder_that_was_never_expanded_has_never_been_fetched() {
    let h = harness(100, 10);
    let _ = h
        .provider
        .read_directory(&h.ws, &RelPath::root(), PageRequest::default())
        .await
        .unwrap();
    // Time passes; the workspace stays open. Nothing else is requested.
    assert_eq!(
        h.engine.call_count("read_directory"),
        1,
        "a client that fetched a whole tree would be unusable on precisely the repositories this \
         product exists for"
    );
}

/// US1.3.
#[tokio::test]
async fn collapsing_and_re_expanding_a_folder_issues_no_further_listing() {
    let h = harness(10, 5);
    let dir = RelPath::parse("/dir00000").unwrap();

    let _ = h
        .provider
        .read_directory(&h.ws, &dir, PageRequest::default())
        .await
        .unwrap();
    assert_eq!(h.engine.call_count("read_directory"), 1);

    // Collapse is a UI act with no provider call; expanding again consults the projection.
    let again = h
        .provider
        .read_directory(&h.ws, &dir, PageRequest::default())
        .await
        .unwrap();
    assert_eq!(
        h.engine.call_count("read_directory"),
        1,
        "content already listed must not be re-requested while it remains valid (FR-016)"
    );
    assert_eq!(again.items.len(), 5, "and it must come back complete");
}

/// US1.4, SC-002. The headline: work proportional to what was opened.
#[tokio::test]
async fn expanding_ten_folders_of_a_hundred_thousand_files_issues_ten_listings() {
    // 5,000 directories of 20 files each: a hundred thousand files.
    let h = harness(5_000, 20);

    let _ = h
        .provider
        .read_directory(&h.ws, &RelPath::root(), PageRequest::default())
        .await
        .unwrap();
    for f in 0..10 {
        let dir = RelPath::parse(&format!("/dir{f:05}")).unwrap();
        let _ = h
            .provider
            .read_directory(&h.ws, &dir, PageRequest::default())
            .await
            .unwrap();
    }

    assert_eq!(
        h.engine.call_count("read_directory"),
        11,
        "one for the root and one per folder expanded — the number of listings requested equals \
         the number of folders actually expanded, not the number that exist (SC-002)"
    );
    assert_eq!(
        h.engine.bytes_transferred(),
        0,
        "and browsing transfers no file content at all"
    );
}

/// The root listing is capped at the protocol page size rather than arriving whole.
#[tokio::test]
async fn a_directory_larger_than_one_page_arrives_paged() {
    let engine = Arc::new(FakeWorkspace::new());
    for n in 0..1_500 {
        engine.file(&format!("/f{n:05}.rs"), b"x");
    }
    let page = engine
        .read_directory(
            &WorkspaceId("w".into()),
            &RelPath::root(),
            PageRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        page.items.len(),
        1_000,
        "capped at the protocol maximum (FR-024)"
    );
    assert!(
        page.next_cursor.is_some(),
        "and the caller is told there is more — a listing of a hundred thousand entries at a \
         hundred bytes each is an order of magnitude past §4.1's frame cap, so a single-message \
         listing is undeliverable rather than merely slow"
    );
}

/// FR-024's other half: a caller **can request the next page**, through the caching layer, and
/// every entry of a folder larger than one page reaches it and the cache.
///
/// Found after F012 by reading, not by a report. The first page was cached as though it were the
/// whole folder, so asking for the next page answered with the first one again and no cursor; and
/// storing a later page would have replaced the earlier one, because a listing replaces a parent's
/// children. A folder of more than 1000 entries showed its first 1000, online and offline alike.
#[tokio::test]
async fn every_page_of_a_large_folder_reaches_the_caller_and_the_cache() {
    let h = harness(1, 1_500);
    let dir = RelPath::parse("/dir00000").unwrap();

    let mut names = std::collections::BTreeSet::new();
    let mut page = PageRequest::default();
    for _ in 0..10 {
        let got = h
            .provider
            .read_directory(&h.ws, &dir, page.clone())
            .await
            .unwrap();
        names.extend(got.items.into_iter().map(|e| e.name));
        match got.next_cursor {
            Some(cursor) => page.cursor = Some(cursor),
            None => break,
        }
    }
    assert_eq!(
        names.len(),
        1_500,
        "every entry, each once, following the cursor"
    );

    // And all of them are held: offline, the folder is complete rather than its first page.
    h.conn.set(ConnectionState::Disconnected);
    let mut offline = std::collections::BTreeSet::new();
    let mut page = PageRequest::default();
    for _ in 0..10 {
        let got = h
            .provider
            .read_directory(&h.ws, &dir, page.clone())
            .await
            .unwrap();
        offline.extend(got.items.into_iter().map(|e| e.name));
        match got.next_cursor {
            Some(cursor) => page.cursor = Some(cursor),
            None => break,
        }
    }
    assert_eq!(
        offline.len(),
        1_500,
        "the cached folder is the whole folder"
    );
}

/// A listing that stops after its first page caches nothing for the folder, so offline it is
/// reported unavailable rather than presented as a complete folder of 1000 entries (FR-033's rule
/// for content, applied to a listing). The cached listing of a folder is therefore always the
/// whole folder or nothing.
#[tokio::test]
async fn a_listing_interrupted_after_one_page_caches_nothing() {
    let h = harness(1, 1_500);
    let dir = RelPath::parse("/dir00000").unwrap();

    let first = h
        .provider
        .read_directory(&h.ws, &dir, PageRequest::default())
        .await
        .unwrap();
    assert!(first.next_cursor.is_some());

    h.conn.set(ConnectionState::Disconnected);
    let offline = h
        .provider
        .read_directory(&h.ws, &dir, PageRequest::default())
        .await;
    assert!(
        offline.is_err(),
        "a partial listing was served offline as though it were the folder"
    );
}
