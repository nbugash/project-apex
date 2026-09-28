//! What reconnection does with the work the host has not seen.
//!
//! **Every assertion here reads the outcome, not only the store.** Three of this feature's defects
//! have the same shape: the durable state is correct and the developer is told something false about
//! it. `Failed` and `NotAttempted` both leave the row; `FastForwarded` and `Merged` both delete it.
//! A test that checked the store alone would pass for an implementation that reported the wrong one
//! of each pair, and what the developer does next depends entirely on which they were told.

mod common;

use apex_shell::adapters::outbound::text_merge::DiffyMerge;
use apex_shell::application::ports::text_merge::{MergeOutcome, TextMerge};
use apex_shell::application::ports::workspace_cache::{PendingEdit, WorkspaceCache};
use apex_shell::application::ports::workspace_provider::{
    ProviderError, ProviderResult, WorkspaceProvider,
};
use apex_shell::application::use_cases::reconcile::{Outcome, Reconcile};
use apex_shell::domain::workspace::{
    ByteRange, DirPage, FileChunk, FsMeta, Location, PageRequest, RelPath, Sha256, Workspace,
    WorkspaceId,
};
use async_trait::async_trait;
use common::fake_cache::InMemoryCache;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn path(p: &str) -> RelPath {
    RelPath::parse(p).expect("a valid path")
}

fn chunk(bytes: &[u8]) -> FileChunk {
    FileChunk {
        bytes: bytes.to_vec(),
        sha256: Sha256::of(bytes),
        total_size: bytes.len() as u64,
        range: ByteRange {
            offset: 0,
            length: bytes.len() as u64,
        },
    }
}

/// A host whose answers the test dictates, and which records what it was asked to write.
///
/// Reads and writes are scripted **separately**, because the race FR-020b describes lives between
/// them: a double that derived the write's answer from its own state could not express "the read
/// succeeded and then the host moved".
#[derive(Default)]
struct ScriptedHost {
    /// Path -> content the host holds. Absent means `NotFound`.
    content: Mutex<BTreeMap<String, Vec<u8>>>,
    /// Paths whose next write is refused as stale, whatever the base says.
    refuse_stale: Mutex<Vec<String>>,
    /// Paths whose next read or write reports the connection gone.
    offline_from: Mutex<Option<String>>,
    /// Every write that was attempted, in order, with the bytes it carried.
    writes: Mutex<Vec<(String, Vec<u8>)>>,
    /// When set, every write fails with this reason.
    write_fails: Mutex<Option<String>>,
    /// When set, every read yields to the scheduler first, so two runs genuinely interleave.
    /// Without it a read resolves immediately and "two concurrent runs" run one after the other,
    /// which would let the in-flight guard's test pass with no guard at all.
    yield_on_read: std::sync::atomic::AtomicBool,
}

impl ScriptedHost {
    fn holding(files: &[(&str, &[u8])]) -> Self {
        let me = Self::default();
        for (p, b) in files {
            me.content
                .lock()
                .unwrap()
                .insert((*p).to_string(), b.to_vec());
        }
        me
    }
    fn writes(&self) -> Vec<(String, Vec<u8>)> {
        self.writes.lock().unwrap().clone()
    }
    fn wrote(&self, p: &str) -> Option<Vec<u8>> {
        self.writes()
            .into_iter()
            .find(|(w, _)| w == p)
            .map(|(_, b)| b)
    }
}

#[async_trait]
impl WorkspaceProvider for ScriptedHost {
    async fn read_directory(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _page: PageRequest,
    ) -> ProviderResult<DirPage> {
        unreachable!("reconciliation lists nothing")
    }

    async fn stat(&self, _ws: &WorkspaceId, _path: &RelPath) -> ProviderResult<FsMeta> {
        unreachable!("reconciliation asks for content, never for metadata")
    }

    async fn read_file(
        &self,
        _ws: &WorkspaceId,
        path: &RelPath,
        _range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        if self
            .yield_on_read
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            tokio::task::yield_now().await;
        }
        if self.offline_from.lock().unwrap().as_deref() == Some(path.as_str()) {
            return Err(ProviderError::Offline);
        }
        match self.content.lock().unwrap().get(path.as_str()) {
            Some(bytes) => Ok(chunk(bytes)),
            None => Err(ProviderError::NotFound),
        }
    }

    async fn write_file(
        &self,
        _ws: &WorkspaceId,
        path: &RelPath,
        content: &[u8],
        _base: &Sha256,
    ) -> ProviderResult<Sha256> {
        if self.offline_from.lock().unwrap().as_deref() == Some(path.as_str()) {
            return Err(ProviderError::Offline);
        }
        self.writes
            .lock()
            .unwrap()
            .push((path.as_str().to_string(), content.to_vec()));
        if let Some(why) = self.write_fails.lock().unwrap().clone() {
            // `Refused` carries no message in this port; the reason travels in the log and in the
            // outcome's own text, which is what `Transport` is for here.
            let _ = why;
            return Err(ProviderError::Transport("disk full on the host".into()));
        }
        // Scripted independently of the read, which is the whole point of this double.
        let mut stale = self.refuse_stale.lock().unwrap();
        if let Some(i) = stale.iter().position(|p| p == path.as_str()) {
            stale.remove(i);
            return Err(ProviderError::WriteConflict);
        }
        self.content
            .lock()
            .unwrap()
            .insert(path.as_str().to_string(), content.to_vec());
        Ok(Sha256::of(content))
    }
}

/// A cache, a host, and a reconciler over both.
fn fixture(host: Arc<ScriptedHost>) -> (Arc<InMemoryCache>, Reconcile, WorkspaceId) {
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
    let reconcile = Reconcile::new(cache.clone(), host, Arc::new(DiffyMerge::new()));
    (cache, reconcile, ws.id)
}

/// Retain work **and** cache the base, which is what a file the developer opened online looks like.
///
/// Most tests use this rather than `retain`, because it is the realistic state: the base is in
/// `file_contents` as well as in the pending edit, so an implementation reading the base from the
/// cache would behave identically. That is what makes the eviction test below able to *locate* the
/// defect rather than merely notice it -- quickstart §5's mutation 9 must fail that test and not
/// these, or the suite says only "something is wrong somewhere".
fn retain_cached(
    cache: &InMemoryCache,
    ws: &WorkspaceId,
    p: &str,
    local: &[u8],
    base: &[u8],
    at: i64,
) {
    use apex_shell::application::ports::workspace_cache::StoreOutcome;
    use apex_shell::domain::workspace::{EntryKind, FsEntry};
    let name = p.trim_start_matches('/');
    // **Accumulated, not replaced.** `put_listing` replaces a parent's children atomically, so
    // calling it once per file wiped the previous file's row and its cached content -- which made
    // every multi-file test quietly uncached and cost mutation 9 its precision. Reading the current
    // children and re-listing with this entry appended is both correct and what a real tree holds.
    let mut entries = cache
        .list_children(ws, &RelPath::root())
        .expect("children")
        .into_iter()
        .map(|e| FsEntry {
            name: e.name,
            kind: e.kind,
            size: e.size,
            modified: e.modified,
        })
        .collect::<Vec<_>>();
    if !entries.iter().any(|e| e.name == name) {
        entries.push(FsEntry {
            name: name.to_string(),
            kind: EntryKind::File,
            size: base.len() as u64,
            modified: 0,
        });
    }
    cache
        .put_listing(ws, &RelPath::root(), &entries)
        .expect("listing");
    let id = cache
        .file_id(ws, &path(p))
        .expect("ask")
        .expect("the file is listed");
    assert!(
        matches!(
            cache.put_content(&id, base, &Sha256::of(base), 0),
            StoreOutcome::Stored
        ),
        "the base must really be cached, or this helper is the same as `retain`"
    );
    retain(cache, ws, p, local, Some(base), at);
}

fn retain(
    cache: &InMemoryCache,
    ws: &WorkspaceId,
    p: &str,
    local: &[u8],
    base: Option<&[u8]>,
    at: i64,
) {
    cache
        .retain_edit(
            ws,
            &path(p),
            &PendingEdit {
                content: local.to_vec(),
                base: base.map(|b| (b.to_vec(), Sha256::of(b))),
                mergeable: true,
                retained_at: at,
            },
        )
        .expect("retain");
}

/// US3 scenario 1, FR-018, FR-019: the host has not moved, so the local content is written as is.
#[tokio::test]
async fn an_unmoved_host_fast_forwards_with_no_interaction() {
    let host = Arc::new(ScriptedHost::holding(&[("/a.rs", b"base\n")]));
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(&cache, &ws, "/a.rs", b"mine\n", b"base\n", 1);

    let report = reconcile.run(&ws).await;

    // The outcome must be FastForwarded and not Merged. Both succeed and both delete the row, so
    // nothing else here distinguishes them -- and reporting a merge for a fast-forward sends a
    // developer to review a combination that never occurred (FR-024).
    assert_eq!(report.files, vec![(path("/a.rs"), Outcome::FastForwarded)]);
    assert_eq!(host.wrote("/a.rs").as_deref(), Some(&b"mine\n"[..]));
    assert!(
        cache.pending_edits(&ws).expect("read").is_empty(),
        "a confirmed write drops the row"
    );
    assert!(!report.has_conflicts(), "no interaction was needed");
}

/// US3 scenario 2, FR-020: a host change elsewhere in the file combines.
#[tokio::test]
async fn a_non_overlapping_host_change_merges_with_no_prompt() {
    // The host changed the last line; the developer changed the first. Two lines apart, which
    // research.md's measurement says must merge cleanly or the feature is useless.
    let host = Arc::new(ScriptedHost::holding(&[("/a.rs", b"a\nb\nc\nd\nE\n")]));
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(
        &cache,
        &ws,
        "/a.rs",
        b"A\nb\nc\nd\ne\n",
        b"a\nb\nc\nd\ne\n",
        1,
    );

    let report = reconcile.run(&ws).await;

    assert_eq!(report.files, vec![(path("/a.rs"), Outcome::Merged)]);
    let written = host.wrote("/a.rs").expect("a merged write");
    let text = String::from_utf8(written).expect("text");
    assert!(
        text.contains('A'),
        "the developer's change survives: {text:?}"
    );
    assert!(text.contains('E'), "the host's change survives: {text:?}");
    assert!(!report.has_conflicts());
}

/// US3 scenario 3, FR-023: per file, so one conflict does not hold back the rest.
#[tokio::test]
async fn reconciliation_is_per_file() {
    let host = Arc::new(ScriptedHost::holding(&[
        ("/clean.rs", b"base\n"),
        ("/collides.rs", b"a\nTHEIRS\nc\n"),
    ]));
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(&cache, &ws, "/clean.rs", b"mine\n", b"base\n", 1);
    retain(
        &cache,
        &ws,
        "/collides.rs",
        b"a\nMINE\nc\n",
        Some(b"a\nb\nc\n"),
        2,
    );

    let report = reconcile.run(&ws).await;

    assert_eq!(
        report.files,
        vec![
            (path("/clean.rs"), Outcome::FastForwarded),
            (path("/collides.rs"), Outcome::Conflicted),
        ]
    );
    let left: Vec<String> = cache
        .pending_edits(&ws)
        .expect("read")
        .into_iter()
        .map(|(p, _)| p.as_str().to_string())
        .collect();
    assert_eq!(
        left,
        vec!["/collides.rs".to_string()],
        "the file that reached the host is no longer pending; the one that did not still is"
    );
}

/// US3 scenario 4, FR-028: an interruption between two files loses nothing and says so.
#[tokio::test]
async fn an_interruption_between_files_reports_not_attempted() {
    let host = Arc::new(ScriptedHost::holding(&[
        ("/first.rs", b"base\n"),
        ("/second.rs", b"base\n"),
    ]));
    *host.offline_from.lock().unwrap() = Some("/second.rs".to_string());
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(&cache, &ws, "/first.rs", b"mine\n", b"base\n", 1);
    retain_cached(&cache, &ws, "/second.rs", b"mine\n", b"base\n", 2);

    let report = reconcile.run(&ws).await;

    // `NotAttempted`, not `Failed`. Both leave the row, so the store cannot tell them apart -- and
    // they mean opposite things: NotAttempted is retried on the next reconnection, Failed is not.
    assert_eq!(
        report.files,
        vec![
            (path("/first.rs"), Outcome::FastForwarded),
            (path("/second.rs"), Outcome::NotAttempted),
        ]
    );
    let left: Vec<String> = cache
        .pending_edits(&ws)
        .expect("read")
        .into_iter()
        .map(|(p, _)| p.as_str().to_string())
        .collect();
    assert_eq!(
        left,
        vec!["/second.rs".to_string()],
        "the unwritten edit stays"
    );
    assert_eq!(host.writes().len(), 1, "no file was partly written");
}

/// FR-011b and edge case EC-08: an evicted cache does not stop a merge.
///
/// The eviction is **real** -- the `file_contents` row is genuinely gone -- or this passes against
/// an implementation that reads the base from the cache, which is the hole analysis run 2 found.
#[tokio::test]
async fn a_file_whose_cached_content_was_evicted_still_merges() {
    let host = Arc::new(ScriptedHost::holding(&[("/a.rs", b"a\nb\nc\nd\nE\n")]));
    let (cache, reconcile, ws) = fixture(host.clone());
    retain(
        &cache,
        &ws,
        "/a.rs",
        b"A\nb\nc\nd\ne\n",
        Some(b"a\nb\nc\nd\ne\n"),
        1,
    );

    // Nothing was ever cached for this path: no `put_listing`, no `put_content`. That is the same
    // state an eviction leaves, and it is the state the base must survive.
    assert!(
        cache.lookup(&ws, &path("/a.rs")).expect("look").is_none(),
        "the cache must be empty for this path, or the test proves nothing about eviction"
    );

    let report = reconcile.run(&ws).await;

    assert_eq!(report.files, vec![(path("/a.rs"), Outcome::Merged)]);
    let text = String::from_utf8(host.wrote("/a.rs").expect("written")).expect("text");
    assert!(
        text.contains('A') && text.contains('E'),
        "the base travelled with the pending edit: {text:?}"
    );
}

/// FR-020b: the host moving between the read and the write is a conflict, not a failure.
#[tokio::test]
async fn a_stale_base_refusal_is_reported_as_a_conflict() {
    let host = Arc::new(ScriptedHost::holding(&[("/a.rs", b"base\n")]));
    // The read succeeds and the write is then refused: the host moved in between. Scripted this way
    // because changing the host *before* reconciliation starts exercises the ordinary overlap path
    // instead, and that passes either way.
    host.refuse_stale.lock().unwrap().push("/a.rs".to_string());
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(&cache, &ws, "/a.rs", b"mine\n", b"base\n", 1);

    let report = reconcile.run(&ws).await;

    // The assertion is the **outcome**. `Failed` keeps the row too, so reading the store alone
    // passes either way -- and the defect is a developer handed an error they cannot act on where a
    // conflict they can resolve was available.
    assert_eq!(report.files, vec![(path("/a.rs"), Outcome::Conflicted)]);
    assert_eq!(
        cache.pending_edits(&ws).expect("read").len(),
        1,
        "the work stays until the developer settles it"
    );
}

/// FR-022: a row is deleted only where the host confirmed a write.
///
/// Driven by failing the write *after* the merge succeeded, which is the ordering a careless
/// implementation gets wrong -- it deletes the row when it decides to write rather than when the
/// write is confirmed.
#[tokio::test]
async fn a_row_survives_a_write_that_was_not_confirmed() {
    let host = Arc::new(ScriptedHost::holding(&[("/a.rs", b"base\n")]));
    *host.write_fails.lock().unwrap() = Some("disk full on the host".to_string());
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(&cache, &ws, "/a.rs", b"mine\n", b"base\n", 1);

    let report = reconcile.run(&ws).await;

    assert!(
        matches!(report.files[0].1, Outcome::Failed(ref why) if why.contains("disk full")),
        "the host's reason reaches the developer: {:?}",
        report.files[0].1
    );
    assert_eq!(
        cache.pending_edits(&ws).expect("read").len(),
        1,
        "the write was attempted and not confirmed, so the work stays (FR-022)"
    );
    assert_eq!(host.writes().len(), 1, "it really did try");
}

/// FR-025a: a file the client cannot merge always prompts, even where the host has not moved.
#[tokio::test]
async fn an_unmergeable_file_prompts_even_when_the_host_has_not_moved() {
    let host = Arc::new(ScriptedHost::holding(&[("/blob.bin", b"base\n")]));
    let (cache, reconcile, ws) = fixture(host.clone());
    cache
        .retain_edit(
            &ws,
            &path("/blob.bin"),
            &PendingEdit {
                content: b"mine\n".to_vec(),
                base: Some((b"base\n".to_vec(), Sha256::of(b"base\n"))),
                mergeable: false,
                retained_at: 1,
            },
        )
        .expect("retain");

    let report = reconcile.run(&ws).await;

    // The host is untouched, so a "prompt only when the remote moved" implementation would
    // fast-forward this and pass every other test in the file.
    assert_eq!(report.files, vec![(path("/blob.bin"), Outcome::Conflicted)]);
    assert!(host.writes().is_empty(), "nothing may be written for it");
    assert_eq!(cache.pending_edits(&ws).expect("read").len(), 1);
}

/// Edge case EC-01 and FR-026: a file the host deleted is a question, not a failure.
#[tokio::test]
async fn a_file_deleted_on_the_host_prompts() {
    // The host holds nothing for this path.
    let host = Arc::new(ScriptedHost::holding(&[]));
    let (cache, reconcile, ws) = fixture(host.clone());
    retain(&cache, &ws, "/gone.rs", b"mine\n", Some(b"base\n"), 1);

    let report = reconcile.run(&ws).await;

    assert_eq!(report.files, vec![(path("/gone.rs"), Outcome::Conflicted)]);
    assert!(
        host.writes().is_empty(),
        "neither the deletion nor the edit may win silently (FR-026)"
    );
}

/// A file created offline is written, with no base to be conditional on.
#[tokio::test]
async fn a_file_created_offline_is_written() {
    let host = Arc::new(ScriptedHost::holding(&[]));
    let (cache, reconcile, ws) = fixture(host.clone());
    retain(&cache, &ws, "/new.rs", b"fresh\n", None, 1);

    let report = reconcile.run(&ws).await;

    assert_eq!(report.files, vec![(path("/new.rs"), Outcome::Merged)]);
    assert_eq!(host.wrote("/new.rs").as_deref(), Some(&b"fresh\n"[..]));
    assert!(cache.pending_edits(&ws).expect("read").is_empty());
}

/// Edge case EC-03: the workspace root being gone reports per file rather than emptying the tree.
#[tokio::test]
async fn a_missing_root_reports_per_file() {
    // Every read says the path is not there, which is what a deleted root looks like from here.
    let host = Arc::new(ScriptedHost::holding(&[]));
    let (cache, reconcile, ws) = fixture(host.clone());
    retain(&cache, &ws, "/a.rs", b"mine\n", Some(b"base\n"), 1);
    retain(&cache, &ws, "/b.rs", b"mine\n", Some(b"base\n"), 2);

    let report = reconcile.run(&ws).await;

    assert_eq!(
        report.files.len(),
        2,
        "per file, not one verdict for the run"
    );
    assert!(
        cache.pending_edits(&ws).expect("read").len() == 2,
        "nothing is discarded because the root went away"
    );
}

/// Edge case EC-05: a run that was cut short resumes, with every unwritten edit intact.
#[tokio::test]
async fn an_interrupted_run_resumes_on_the_next_reconnection() {
    let host = Arc::new(ScriptedHost::holding(&[
        ("/first.rs", b"base\n"),
        ("/second.rs", b"base\n"),
    ]));
    *host.offline_from.lock().unwrap() = Some("/second.rs".to_string());
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(&cache, &ws, "/first.rs", b"mine\n", b"base\n", 1);
    retain_cached(&cache, &ws, "/second.rs", b"second\n", b"base\n", 2);

    let first = reconcile.run(&ws).await;
    assert_eq!(first.files[1].1, Outcome::NotAttempted);

    // The connection returns. Nothing else changed, and the second file is still waiting.
    *host.offline_from.lock().unwrap() = None;
    let second = reconcile.run(&ws).await;

    assert_eq!(
        second.files,
        vec![(path("/second.rs"), Outcome::FastForwarded)]
    );
    assert_eq!(host.wrote("/second.rs").as_deref(), Some(&b"second\n"[..]));
    assert!(cache.pending_edits(&ws).expect("read").is_empty());
}

/// The merge is reached through the port, so a different implementation changes the decision.
///
/// Not a test of `diffy`: that is `merge_agreement.rs`. This asserts the reconciler *asks* rather
/// than deciding for itself, which is what makes the conflict boundary replaceable and what
/// Principle VIII requires of a use case.
#[tokio::test]
async fn the_reconciler_asks_the_port_rather_than_deciding() {
    struct AlwaysConflict;
    impl TextMerge for AlwaysConflict {
        fn merge(&self, _: &str, _: &str, _: &str) -> MergeOutcome {
            MergeOutcome::Conflict
        }
    }
    let host = Arc::new(ScriptedHost::holding(&[("/a.rs", b"a\nb\nc\nd\nE\n")]));
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
    retain(
        &cache,
        &ws.id,
        "/a.rs",
        b"A\nb\nc\nd\ne\n",
        Some(b"a\nb\nc\nd\ne\n"),
        1,
    );

    let reconcile = Reconcile::new(cache.clone(), host.clone(), Arc::new(AlwaysConflict));
    let report = reconcile.run(&ws.id).await;

    // The same inputs merge cleanly under `DiffyMerge` -- asserted two tests above -- so a
    // reconciler that had its own opinion would report `Merged` here.
    assert_eq!(report.files, vec![(path("/a.rs"), Outcome::Conflicted)]);
    assert!(host.writes().is_empty());
}

// ---- when reconciliation starts (A-RECONNECT, EC-16) ----

use apex_shell::application::use_cases::reconcile::entered_connected;
use apex_shell::domain::connection::ConnectionState as CS;

/// Once per transition into `Connected`, and on no other state.
#[test]
fn reconciliation_starts_only_on_entering_connected() {
    // The transition that starts it.
    assert!(entered_connected(&CS::Disconnected, &CS::Connected));
    assert!(entered_connected(&CS::Connecting, &CS::Connected));
    assert!(entered_connected(&CS::Unknown, &CS::Connected));

    // Already connected: the sink is invoked with the current state on subscribe and on every
    // change, so without this a reconnection reporting `Connected` twice would reconcile twice --
    // reading every file again and writing nothing.
    assert!(!entered_connected(&CS::Connected, &CS::Connected));

    // Not connected. Reconciling during `Connecting` would write against a host that is not there,
    // and **EC-16 holds here by construction**: a reconnection whose protocol version is
    // incompatible never reaches `Connected` at all, because §3.8 refuses a newer engine before the
    // link is usable. A workspace that cannot be used cannot be reconciled, and nothing needs to
    // check for that separately.
    for state in [CS::Disconnected, CS::Connecting, CS::Unknown] {
        assert!(
            !entered_connected(&CS::Connected, &state),
            "{state:?} must not start a reconciliation"
        );
        assert!(!entered_connected(&CS::Disconnected, &state));
    }
}

/// Two triggers, one run. The second returns empty and nothing is written twice.
///
/// There are two triggers now -- a transition into `Connected`, and a workspace opened or resumed
/// while connected -- and a workspace opened during a reconnection's run would otherwise start a
/// second run reading the rows the first is writing.
#[tokio::test]
async fn an_overlapping_run_declines_rather_than_writing_twice() {
    let host = Arc::new(ScriptedHost::holding(&[("/a.rs", b"base\n")]));
    host.yield_on_read
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (cache, reconcile, ws) = fixture(host.clone());
    retain_cached(&cache, &ws, "/a.rs", b"mine\n", b"base\n", 1);

    let (first, second) = tokio::join!(reconcile.run(&ws), reconcile.run(&ws));

    let writes = host.writes().iter().filter(|(p, _)| p == "/a.rs").count();
    assert_eq!(
        writes, 1,
        "the file must be written once, not once per trigger"
    );
    let files = first.files.len() + second.files.len();
    assert_eq!(
        files, 1,
        "exactly one run reports the file; the other declined"
    );
    assert!(cache.pending_edits(&ws).expect("read").is_empty());
}
