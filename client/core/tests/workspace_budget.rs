//! §1.4's two sidebar targets, measured rather than asserted (Principle V, A-NFR).
//!
//! p99 over at least 100 samples, at the boundary between the interface and the transport, with
//! any harness-injected delay excluded. **The measured value is printed, not merely compared**:
//! a budget only ever compared against tells nobody how much headroom is left, which is what
//! says whether the next feature's work can be afforded.

mod common;

use apex_shell::adapters::outbound::sqlite::{schema, SqliteWorkspaceCache};
use apex_shell::adapters::outbound::stub_connection::StubConnectionStatusSource;
use apex_shell::application::ports::workspace_cache::{StoreOutcome, WorkspaceCache};
use apex_shell::application::ports::workspace_provider::WorkspaceProvider;
use apex_shell::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::workspace::{
    EntryKind, FsEntry, Location, PageRequest, RelPath, Sha256, Workspace, WorkspaceId,
};
use common::fake_clock::FakeClock;
use common::fake_workspace::FakeWorkspace;
use std::sync::Arc;
use std::time::Instant;

const SAMPLES: usize = 200;

/// The 99th percentile. Not the mean, which hides exactly the interaction that feels slow, and
/// not the max, which fails a build on one scheduler hiccup.
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

/// A real database with `folders` directories, each holding `per` files.
fn populated(
    path: &std::path::Path,
    folders: usize,
    per: usize,
) -> (Arc<SqliteWorkspaceCache>, WorkspaceId) {
    let cache = Arc::new(SqliteWorkspaceCache::open(path).expect("open"));
    cache
        .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("v1");
    let ws = workspace("w1");
    cache.register(&ws, 0).unwrap();

    let roots: Vec<FsEntry> = (0..folders)
        .map(|f| FsEntry {
            name: format!("dir{f:05}"),
            kind: EntryKind::Directory,
            size: 0,
            modified: 0,
        })
        .collect();
    cache.put_listing(&ws.id, &RelPath::root(), &roots).unwrap();

    for f in 0..folders {
        let dir = RelPath::parse(&format!("/dir{f:05}")).unwrap();
        let children: Vec<FsEntry> = (0..per)
            .map(|n| FsEntry {
                name: format!("file{n:05}.rs"),
                kind: EntryKind::File,
                size: 100,
                modified: 0,
            })
            .collect();
        cache.put_listing(&ws.id, &dir, &children).unwrap();
    }
    (cache, ws.id)
}

/// §1.4: "Sidebar folder expand (cached) — < 1 ms. Served from local SQLite."
#[tokio::test]
async fn a_cached_folder_expand_stays_inside_one_millisecond_at_p99() {
    let dir = tempfile::tempdir().unwrap();
    // A realistic workspace: a thousand folders of forty files, so the index is doing real work
    // rather than answering from a table that fits in a cache line.
    let (cache, ws) = populated(&dir.path().join("cache.db"), 1_000, 40);

    let conn = StubConnectionStatusSource::new();
    conn.set(ConnectionState::Connected);
    let provider = CachedWorkspace::new(
        Arc::new(FakeWorkspace::new()),
        cache,
        Arc::new(FakeClock::at(0)),
        Arc::new(conn),
        Arc::new(|_| {}),
        Limits::default(),
    );

    let mut samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // A different folder each time, so nothing is answered from a warm row cache that a
        // real developer browsing a tree would not have.
        let path = RelPath::parse(&format!("/dir{:05}", i % 1_000)).unwrap();
        let start = Instant::now();
        let page = provider
            .read_directory(&ws, &path, PageRequest::default())
            .await
            .expect("cached expand");
        samples.push(start.elapsed().as_micros());
        assert_eq!(page.items.len(), 40);
    }

    let measured = p99(samples);
    println!("sidebar expand (cached)    p99 = {measured} us   budget 1000 us");
    assert!(
        measured < 1_000,
        "§1.4 budgets a cached folder expand at under 1 ms; measured {measured} us at p99. The \
         whole architecture — thin client, remote engine, local projection — exists to buy this."
    );
}

/// §1.4: "Sidebar folder expand (uncached) — < 250 ms. One shallow workspace/readDirectory."
///
/// Measured at the boundary between the interface and the transport, so the double's own
/// latency is what a real network would replace and is deliberately zero here: this measures
/// what *the system* adds, which is the distinction A-NFR draws between a performance gate and
/// a number.
#[tokio::test]
async fn an_uncached_folder_expand_stays_inside_the_interaction_budget_at_p99() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(SqliteWorkspaceCache::open(&dir.path().join("cache.db")).expect("open"));
    cache
        .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("v1");
    let ws = workspace("w1");
    cache.register(&ws, 0).unwrap();

    let engine = Arc::new(FakeWorkspace::new());
    for f in 0..SAMPLES {
        for n in 0..40 {
            engine.file(&format!("/dir{f:05}/file{n:05}.rs"), b"x");
        }
    }

    let conn = StubConnectionStatusSource::new();
    conn.set(ConnectionState::Connected);
    let provider = CachedWorkspace::new(
        engine,
        cache,
        Arc::new(FakeClock::at(0)),
        Arc::new(conn),
        Arc::new(|_| {}),
        Limits::default(),
    );

    let mut samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // Each folder is fetched for the first time: a genuine miss, including the write that
        // persists the listing.
        let path = RelPath::parse(&format!("/dir{i:05}")).unwrap();
        let start = Instant::now();
        let page = provider
            .read_directory(&ws.id, &path, PageRequest::default())
            .await
            .expect("uncached expand");
        samples.push(start.elapsed().as_micros());
        assert_eq!(page.items.len(), 40);
    }

    let measured = p99(samples);
    println!("sidebar expand (uncached)  p99 = {measured} us   budget 250000 us");
    assert!(
        measured < 250_000,
        "§1.4 budgets an uncached folder expand at under 250 ms; measured {measured} us at p99"
    );
}

/// SC-010: cached content occupies at most half the disk of what it represents, measured over a
/// real source tree rather than generated text — which compresses far better than code and would
/// make the budget meaningless.
#[test]
fn compressed_content_is_at_most_half_the_size_of_what_it_represents() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SqliteWorkspaceCache::open(&dir.path().join("cache.db")).expect("open");
    cache
        .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("v1");
    let ws = workspace("w1");
    cache.register(&ws, 0).unwrap();

    // This repository's own Rust sources: real code, with real comments and real repetition.
    // `cargo test` runs with the package root as the working directory, not the workspace root,
    // so the tree is located from the manifest rather than from a relative path that depends on
    // where the runner happened to start.
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for root in ["client/core/src", "engine/src", "protocol/src"] {
        collect(&repo.join(root), &mut files);
    }
    // Without this the measurement below would divide zero by zero and report a ratio that
    // passes while proving nothing — which is how a relative path that stopped resolving would
    // have turned a gate into decoration.
    assert!(
        files.len() > 20,
        "expected a real source tree, found {} files",
        files.len()
    );

    let entries: Vec<FsEntry> = files
        .iter()
        .enumerate()
        .map(|(i, (_, bytes))| FsEntry {
            name: format!("f{i:04}.rs"),
            kind: EntryKind::File,
            size: bytes.len() as u64,
            modified: 0,
        })
        .collect();
    cache
        .put_listing(&ws.id, &RelPath::root(), &entries)
        .unwrap();

    let mut raw = 0u64;
    for (i, (_, bytes)) in files.iter().enumerate() {
        let path = RelPath::parse(&format!("/f{i:04}.rs")).unwrap();
        let id = cache.file_id(&ws.id, &path).unwrap().unwrap();
        assert_eq!(
            cache.put_content(&id, bytes, &Sha256::of(bytes), 0),
            StoreOutcome::Stored
        );
        raw += bytes.len() as u64;
    }

    let stored = cache.stored_content_bytes().expect("blob total");

    let ratio = stored as f64 / raw as f64 * 100.0;
    println!(
        "compression ratio          {ratio:.1} %        budget 50.0 %   ({stored} of {raw} bytes)"
    );
    assert!(
        ratio <= 50.0,
        "SC-010 budgets cached content at half the disk of what it represents; measured {ratio:.1}%"
    );
}

fn collect(dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            if let Ok(bytes) = std::fs::read(&p) {
                out.push((p.display().to_string(), bytes));
            }
        }
    }
}
