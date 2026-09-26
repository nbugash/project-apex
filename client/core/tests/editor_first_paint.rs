//! SC-005, SC-006 and SC-007: opening a large file, measured rather than asserted.
//!
//! p99 over at least 100 samples, at the boundary between the interface and the transport, with
//! the double's own latency deliberately zero — what is being measured is what *the system*
//! adds, which is the distinction A-NFR draws between a performance gate and a number.
//!
//! **Every measured value is printed.** A budget only ever compared against tells nobody how
//! much headroom is left, which is what says whether the next feature's work can be afforded.

mod common;

use apex_shell::adapters::outbound::sqlite::{schema, SqliteWorkspaceCache};
use apex_shell::adapters::outbound::stub_connection::StubConnectionStatusSource;
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::ports::workspace_provider::WorkspaceProvider;
use apex_shell::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::workspace::{ByteRange, Location, RelPath, Workspace, WorkspaceId};
use common::fake_clock::FakeClock;
use common::fake_workspace::FakeWorkspace;
use std::sync::Arc;
use std::time::Instant;

const SAMPLES: usize = 200;

/// One window, as plan.md fixes it.
const SCROLL_RANGE: u64 = 256 * 1024;
/// §4.1's cap. No single response may exceed it.
const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// A file well past the chunk threshold, so the windowed path is the one under test.
const BIG: usize = 4 * 1024 * 1024;

fn p99(mut us: Vec<u128>) -> u128 {
    us.sort_unstable();
    us[(us.len() as f64 * 0.99) as usize - 1]
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

fn subject(inner: Arc<FakeWorkspace>, dir: &std::path::Path) -> (CachedWorkspace, WorkspaceId) {
    let cache = Arc::new(SqliteWorkspaceCache::open(&dir.join("cache.db")).expect("open"));
    cache
        .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("migrate");
    let ws = workspace("w1");
    cache.register(&ws, 0).unwrap();

    let conn = StubConnectionStatusSource::new();
    conn.set(ConnectionState::Connected);
    (
        CachedWorkspace::new(
            inner,
            cache,
            Arc::new(FakeClock::at(0)),
            Arc::new(conn),
            Arc::new(|_| {}),
            Limits::default(),
        ),
        ws.id,
    )
}

#[tokio::test]
async fn the_first_window_of_a_large_file_arrives_inside_the_interaction_budget() {
    // SC-005. What a developer waits for is the first screenful, not the file: a four megabyte
    // log must show its beginning as quickly as a four kilobyte one, because the window that
    // appears is the same size either way.
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeWorkspace::new());
    fake.file("/big.log", &vec![b'x'; BIG]);
    let (provider, ws) = subject(fake.clone(), dir.path());
    let path = RelPath::parse("/big.log").unwrap();

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let chunk = provider
            .read_file(
                &ws,
                &path,
                Some(ByteRange {
                    offset: 0,
                    length: SCROLL_RANGE,
                }),
            )
            .await
            .expect("first window");
        samples.push(start.elapsed().as_micros());
        assert_eq!(chunk.bytes.len() as u64, SCROLL_RANGE);
        assert_eq!(chunk.total_size, BIG as u64);
    }

    let measured = p99(samples);
    // The figure **includes** the double's own cost, which is not small: `FakeWorkspace` digests
    // the whole four megabytes on every call, where a real engine digests it once and serves the
    // window from an open file. So the number printed is an upper bound on what the system adds,
    // and the budget is met with the harness's own work counted against it. Said plainly rather
    // than quietly subtracted, because a measurement whose method is not stated is a number.
    println!("editor first window        p99 = {measured} us   budget 250000 us (includes the double's whole-file digest)");
    assert!(
        measured < 250_000,
        "§1.4 budgets an interaction at under 250 ms; measured {measured} us at p99"
    );
}

#[tokio::test]
async fn no_single_response_exceeds_the_frame_limit() {
    // SC-007. Asserted on what came back rather than on what was asked for: a request for a
    // legal amount can still be answered with more than fits if anything miscounts, and §4.1's
    // cap is about the frame carrying the answer.
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeWorkspace::new());
    fake.file("/big.log", &vec![b'x'; BIG]);
    let (provider, ws) = subject(fake, dir.path());
    let path = RelPath::parse("/big.log").unwrap();

    let mut largest = 0usize;
    let mut offset = 0u64;
    while offset < BIG as u64 {
        let chunk = provider
            .read_file(
                &ws,
                &path,
                Some(ByteRange {
                    offset,
                    length: SCROLL_RANGE,
                }),
            )
            .await
            .expect("window");
        largest = largest.max(chunk.bytes.len());
        offset += chunk.range.length.max(1);
    }

    println!("editor largest response    = {largest} bytes   limit {MAX_FRAME_BYTES} bytes");
    assert!(
        largest <= MAX_FRAME_BYTES,
        "a response of {largest} bytes cannot be framed; §4.1 caps a frame at {MAX_FRAME_BYTES}"
    );
}

#[tokio::test]
async fn a_whole_large_file_costs_a_bounded_number_of_windows() {
    // SC-006. The count is the point: an implementation that fetched a kilobyte at a time would
    // satisfy every other assertion here and take four thousand round trips to read one file.
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeWorkspace::new());
    fake.file("/big.log", &vec![b'x'; BIG]);
    let (provider, ws) = subject(fake.clone(), dir.path());
    let path = RelPath::parse("/big.log").unwrap();

    let mut offset = 0u64;
    while offset < BIG as u64 {
        let chunk = provider
            .read_file(
                &ws,
                &path,
                Some(ByteRange {
                    offset,
                    length: SCROLL_RANGE,
                }),
            )
            .await
            .expect("window");
        offset += chunk.range.length.max(1);
    }

    let windows = fake.call_count("read_file");
    let expected = BIG.div_ceil(SCROLL_RANGE as usize);
    println!("editor windows for {BIG} bytes = {windows}   ideal {expected}");
    assert_eq!(
        windows, expected,
        "each window must be one request: {windows} for a file that needs {expected}"
    );
    assert_eq!(
        fake.bytes_transferred(),
        BIG,
        "the file is transferred once, not once per window"
    );
}
