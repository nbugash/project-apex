//! Prefetch: what is cached before the connection goes, without the developer asking (US5).
//!
//! **Asserted against prefetch's own report**, never against a wall-clock wait (T064): SC-010 is
//! "readable offline once prefetch reports done or stopped", so the report is the measurement and
//! each test then goes offline and reads what the report claims.
//!
//! Driven through a real `CachedWorkspace` over a scripted host, because what prefetch achieves is a
//! property of the caching layer it goes through -- a prefetcher that fetched without that layer
//! storing anything would report success and leave the cache empty.

mod common;

use apex_shell::adapters::outbound::stub_connection::StubConnectionStatusSource;
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WorkspaceProvider,
};
use apex_shell::application::use_cases::cached_workspace::{CachedWorkspace, Limits};
use apex_shell::application::use_cases::prefetch::{Prefetch, PREFETCH_BUDGET_BYTES};
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsMeta, Location, PageRequest, RelPath, Sha256, Workspace,
    WorkspaceId,
};
use async_trait::async_trait;
use common::fake_cache::InMemoryCache;
use common::fake_clock::FakeClock;
use common::fake_git::FakeGit;
use common::fake_workspace::FakeWorkspace;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn path(p: &str) -> RelPath {
    RelPath::parse(p).expect("path")
}

/// A host that goes away after answering `reads` file reads: the connection dropping mid-prefetch.
struct DropsAfter {
    inner: Arc<FakeWorkspace>,
    reads: AtomicUsize,
}

#[async_trait]
impl WorkspaceProvider for DropsAfter {
    async fn read_directory(
        &self,
        ws: &WorkspaceId,
        p: &RelPath,
        page: PageRequest,
    ) -> ProviderResult<DirPage> {
        self.inner.read_directory(ws, p, page).await
    }
    async fn stat(&self, ws: &WorkspaceId, p: &RelPath) -> ProviderResult<FsMeta> {
        self.inner.stat(ws, p).await
    }
    async fn read_file(
        &self,
        ws: &WorkspaceId,
        p: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        // Decremented per read; once it reaches zero every read is the connection gone.
        if self
            .reads
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_err()
        {
            return Err(ProviderError::Offline);
        }
        self.inner.read_file(ws, p, range).await
    }
    async fn write_file(
        &self,
        ws: &WorkspaceId,
        p: &RelPath,
        c: &[u8],
        b: &Sha256,
    ) -> ProviderResult<Sha256> {
        self.inner.write_file(ws, p, c, b).await
    }
}

struct Harness {
    cache: Arc<InMemoryCache>,
    conn: Arc<StubConnectionStatusSource>,
    git: Arc<FakeGit>,
    reader: Arc<CachedWorkspace>,
    prefetch: Prefetch,
    ws: WorkspaceId,
}

fn harness_over(host: Arc<dyn WorkspaceProvider>, budget: u64) -> Harness {
    let cache = Arc::new(InMemoryCache::new());
    let ws = Workspace {
        id: WorkspaceId("w1".into()),
        name: "repo".into(),
        location: Location::Local {
            base: "/repo".into(),
        },
        last_opened_at: 0,
    };
    cache.register(&ws, 0).expect("register");
    let conn = Arc::new(StubConnectionStatusSource::new());
    conn.set(ConnectionState::Connected);
    let reader = Arc::new(CachedWorkspace::new(
        host,
        cache.clone(),
        Arc::new(FakeClock::at(1)),
        conn.clone(),
        Arc::new(|_| {}),
        Limits::default(),
    ));
    let git = Arc::new(FakeGit::new());
    let prefetch =
        Prefetch::new(reader.clone(), git.clone(), cache.clone(), conn.clone()).with_budget(budget);
    Harness {
        cache,
        conn,
        git,
        reader,
        prefetch,
        ws: ws.id,
    }
}

fn a_project() -> Arc<FakeWorkspace> {
    let host = Arc::new(FakeWorkspace::new());
    host.file("/Cargo.toml", b"[package]\nname = \"x\"\n")
        .file("/package.json", b"{}\n")
        .file("/src/deep/recent.rs", b"fn recent() {}\n")
        .file("/README.md", b"# readme\n")
        .file("/src/untouched.rs", b"fn untouched() {}\n");
    host
}

impl Harness {
    /// Readable with the connection gone: the claim SC-010 makes about what prefetch reported.
    async fn readable_offline(&self, p: &str) -> bool {
        self.conn.set(ConnectionState::Disconnected);
        let ok = self
            .reader
            .read_file(&self.ws, &path(p), None)
            .await
            .is_ok();
        self.conn.set(ConnectionState::Connected);
        ok
    }
}

/// US5 scenario 1, FR-029 and SC-010: manifests and the files recent commits touched are cached
/// without being asked for, including a recent file in a folder the tree never listed.
#[tokio::test]
async fn manifests_and_recent_commit_files_are_cached_without_being_asked_for() {
    let host = a_project();
    let h = harness_over(host.clone(), PREFETCH_BUDGET_BYTES);
    h.git
        .set_recent(Ok(vec!["/src/deep/recent.rs".into(), "/README.md".into()]));

    let report = h.prefetch.run(&h.ws).await;

    println!(
        "SC-010 prefetch fetched {} files, stopped at budget: {}",
        report.fetched, report.stopped_at_budget
    );
    assert_eq!(report.fetched, 4, "two manifests and two recent files");
    assert!(!report.stopped_at_budget);
    for p in [
        "/Cargo.toml",
        "/package.json",
        "/src/deep/recent.rs",
        "/README.md",
    ] {
        assert!(h.readable_offline(p).await, "{p} is not readable offline");
    }
    assert!(
        !h.readable_offline("/src/untouched.rs").await,
        "a file neither a manifest nor recent is left alone"
    );
}

/// US5 scenario 3 and FR-032: a workspace with no repository still caches its manifests, and a git
/// that cannot answer at all is not a failed prefetch.
#[tokio::test]
async fn a_workspace_with_no_repository_still_caches_manifests() {
    for answer in [
        Ok(Vec::new()),
        Err(ProviderError::Transport("no git".into())),
    ] {
        let h = harness_over(a_project(), PREFETCH_BUDGET_BYTES);
        h.git.set_recent(answer);

        let report = h.prefetch.run(&h.ws).await;

        assert_eq!(report.fetched, 2, "the two manifests, and nothing else");
        assert!(h.readable_offline("/Cargo.toml").await);
        assert!(h.readable_offline("/package.json").await);
    }
}

/// US5 scenario 5, FR-029a and SC-010a: with the cache at its budget, prefetch stops, evicts
/// nothing, and reports stopping rather than failing.
///
/// The budget is set so the first manifest fits and the second does not, and a file the developer
/// opened earlier -- the least recently used entry -- is in the cache throughout. An implementation
/// that evicted to make room would fetch the second manifest and lose the opened file.
#[tokio::test]
async fn at_its_budget_prefetch_stops_and_evicts_nothing() {
    let host = Arc::new(FakeWorkspace::new());
    let opened = vec![b'o'; 60];
    host.file("/opened.rs", &opened)
        .file("/Cargo.toml", &[b'c'; 30])
        .file("/package.json", &[b'p'; 30]);
    let h = harness_over(host, 100);
    // The developer opened this before prefetch ran, so it is the oldest thing cached.
    h.reader
        .read_directory(&h.ws, &RelPath::root(), PageRequest::default())
        .await
        .expect("list");
    h.reader
        .read_file(&h.ws, &path("/opened.rs"), None)
        .await
        .expect("open");
    assert_eq!(h.cache.cached_bytes().expect("bytes"), 60);

    let report = h.prefetch.run(&h.ws).await;

    assert!(
        report.stopped_at_budget,
        "reported as stopping, not as a failure"
    );
    assert_eq!(
        report.fetched, 1,
        "the first manifest fitted; the second would not have"
    );
    assert!(
        h.readable_offline("/opened.rs").await,
        "the opened file was not displaced"
    );
    assert!(
        h.cache.cached_bytes().expect("bytes") <= 100,
        "the cache is within its budget"
    );
}

/// US5 scenario 4 and FR-031: a prefetch the connection interrupts leaves what completed usable and
/// no half-written entry for what did not.
#[tokio::test]
async fn an_interrupted_prefetch_leaves_no_half_written_entry() {
    let inner = a_project();
    let host = Arc::new(DropsAfter {
        inner,
        reads: AtomicUsize::new(1),
    });
    let h = harness_over(host, PREFETCH_BUDGET_BYTES);
    h.git.set_recent(Ok(vec!["/src/deep/recent.rs".into()]));

    let report = h.prefetch.run(&h.ws).await;

    assert_eq!(
        report.fetched, 1,
        "one read completed before the connection went"
    );
    assert!(!report.stopped_at_budget);
    assert!(
        h.readable_offline("/Cargo.toml").await,
        "what completed is usable"
    );
    for p in ["/package.json", "/src/deep/recent.rs"] {
        assert!(
            h.cache.lookup(&h.ws, &path(p)).expect("lookup").is_none(),
            "{p} has an entry although its read never completed"
        );
    }
}

/// FR-029b: prefetch does not run while disconnected -- it asks the host nothing at all -- and it
/// runs for a workspace registered after the prefetcher was built, which is the case a startup-only
/// implementation fails silently. The reconnection trigger is the sequence's `prefetch` step, tested
/// in `reconnect.rs`.
#[tokio::test]
async fn prefetch_runs_only_while_connected_and_for_any_workspace_opened_later() {
    let host = a_project();
    let h = harness_over(host.clone(), PREFETCH_BUDGET_BYTES);
    h.conn.set(ConnectionState::Disconnected);

    let offline = h.prefetch.run(&h.ws).await;

    assert_eq!(offline.fetched, 0);
    assert_eq!(
        host.total_calls(),
        0,
        "nothing was asked of a host that is not there"
    );
    // Including git. The caching layer already serves offline without asking the host, so with
    // the root listed the git question is the one only prefetch's own gate keeps off a dead link.
    h.conn.set(ConnectionState::Connected);
    h.reader
        .read_directory(&h.ws, &RelPath::root(), PageRequest::default())
        .await
        .expect("list while connected");
    h.conn.set(ConnectionState::Disconnected);
    let _ = h.prefetch.run(&h.ws).await;
    assert_eq!(
        h.git.recent_asked.load(Ordering::SeqCst),
        0,
        "git was asked while disconnected"
    );

    h.conn.set(ConnectionState::Connected);
    let later = Workspace {
        id: WorkspaceId("opened-later".into()),
        name: "later".into(),
        location: Location::Local {
            base: "/later".into(),
        },
        last_opened_at: 0,
    };
    h.cache.register(&later, 0).expect("register");
    let report = h.prefetch.run(&later.id).await;
    assert_eq!(report.fetched, 2, "the later workspace's manifests");
}

/// A host whose file changes after the first range has been read.
struct ChangesMidRead {
    inner: Arc<FakeWorkspace>,
    reads: AtomicUsize,
}

#[async_trait]
impl WorkspaceProvider for ChangesMidRead {
    async fn read_directory(
        &self,
        ws: &WorkspaceId,
        p: &RelPath,
        page: PageRequest,
    ) -> ProviderResult<DirPage> {
        self.inner.read_directory(ws, p, page).await
    }
    async fn stat(&self, ws: &WorkspaceId, p: &RelPath) -> ProviderResult<FsMeta> {
        self.inner.stat(ws, p).await
    }
    async fn read_file(
        &self,
        ws: &WorkspaceId,
        p: &RelPath,
        range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        let chunk = self.inner.read_file(ws, p, range).await;
        if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
            self.inner.rewrite(p.as_str(), &vec![b'z'; 200 * 1024]);
        }
        chunk
    }
    async fn write_file(
        &self,
        ws: &WorkspaceId,
        p: &RelPath,
        c: &[u8],
        b: &Sha256,
    ) -> ProviderResult<Sha256> {
        self.inner.write_file(ws, p, c, b).await
    }
}

/// SC-009's chunked reads, and the risk they bring: a file read in ranges that changes between two
/// of them is discarded rather than cached as a file that never existed.
#[tokio::test]
async fn a_file_that_changes_between_ranges_is_not_cached() {
    let inner = Arc::new(FakeWorkspace::new());
    // Three ranges' worth, so there is a "between".
    inner.file("/Cargo.toml", &vec![b'a'; 150 * 1024]);
    let host = Arc::new(ChangesMidRead {
        inner,
        reads: AtomicUsize::new(0),
    });
    let h = harness_over(host, PREFETCH_BUDGET_BYTES);

    let report = h.prefetch.run(&h.ws).await;

    assert_eq!(report.fetched, 0);
    assert!(h
        .cache
        .lookup(&h.ws, &path("/Cargo.toml"))
        .expect("lookup")
        .is_none());
}
