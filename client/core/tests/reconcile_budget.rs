//! SC-011: reconciling a hundred files completes inside ten seconds.
//!
//! Asserted **and** printed. Principle V asks for a measurement that fails when the budget is
//! exceeded, and printing alone is not that.
//!
//! Against the real store on a real file, not the in-memory double. The reconciliation cost this
//! criterion is about is dominated by reading a hundred rows out of SQLite, decompressing two blobs
//! each and writing a hundred times -- a double returning clones would measure the loop and not the
//! work, which is how F011's first budget test came to print `0 us`.

use apex_shell::adapters::outbound::sqlite::schema::CURRENT_VERSION;
use apex_shell::adapters::outbound::sqlite::SqliteWorkspaceCache;
use apex_shell::adapters::outbound::text_merge::DiffyMerge;
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
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const FILES: usize = 100;
const BUDGET_MS: u128 = 10_000;

/// A host that answers from a map. No network and no sleeping: the budget is about the client's
/// work, and a double that slept would measure the sleep.
#[derive(Default)]
struct MapHost {
    content: Mutex<BTreeMap<String, Vec<u8>>>,
}

#[async_trait]
impl WorkspaceProvider for MapHost {
    async fn read_directory(
        &self,
        _ws: &WorkspaceId,
        _path: &RelPath,
        _page: PageRequest,
    ) -> ProviderResult<DirPage> {
        unreachable!()
    }
    async fn stat(&self, _ws: &WorkspaceId, _path: &RelPath) -> ProviderResult<FsMeta> {
        unreachable!()
    }
    async fn read_file(
        &self,
        _ws: &WorkspaceId,
        path: &RelPath,
        _range: Option<ByteRange>,
    ) -> ProviderResult<FileChunk> {
        match self.content.lock().unwrap().get(path.as_str()) {
            Some(bytes) => Ok(FileChunk {
                bytes: bytes.clone(),
                sha256: Sha256::of(bytes),
                total_size: bytes.len() as u64,
                range: ByteRange {
                    offset: 0,
                    length: bytes.len() as u64,
                },
            }),
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
        self.content
            .lock()
            .unwrap()
            .insert(path.as_str().to_string(), content.to_vec());
        Ok(Sha256::of(content))
    }
}

#[tokio::test]
async fn sc_011_reconciling_a_hundred_files_within_the_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(SqliteWorkspaceCache::open(&dir.path().join("cache.db")).expect("open"));
    store
        .migrate_to(CURRENT_VERSION, &mut |_| {})
        .expect("migrate");
    let ws = Workspace {
        id: WorkspaceId("w1".into()),
        name: "repo".into(),
        location: Location::Local {
            base: "/repo".into(),
        },
        last_opened_at: 0,
    };
    store.register(&ws, 0).expect("register");

    // A realistic mixture, because the budget is for a run and not for a best case: half the files
    // fast-forward, half need a real three-way merge. A hundred fast-forwards would measure the
    // hash comparison and never the merge.
    let host = Arc::new(MapHost::default());
    for i in 0..FILES {
        let path = RelPath::parse(&format!("/src/file_{i}.rs")).expect("path");
        // Twenty lines, which is a small source file and enough for the merge to do work.
        let base: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let local = base.replacen("line 0\n", "LOCAL\n", 1);
        let remote = if i % 2 == 0 {
            base.clone() // unmoved: fast-forward
        } else {
            base.replacen("line 19\n", "REMOTE\n", 1) // moved elsewhere: merge
        };
        host.content
            .lock()
            .unwrap()
            .insert(path.as_str().to_string(), remote.into_bytes());
        store
            .retain_edit(
                &ws.id,
                &path,
                &PendingEdit {
                    content: local.into_bytes(),
                    base: Some((base.clone().into_bytes(), Sha256::of(base.as_bytes()))),
                    mergeable: true,
                    retained_at: i as i64,
                },
            )
            .expect("retain");
    }
    assert_eq!(
        store.pending_edits(&ws.id).expect("read").len(),
        FILES,
        "the fixture must really hold a hundred edits"
    );

    let reconcile = Reconcile::new(store.clone(), host.clone(), Arc::new(DiffyMerge::new()));

    let at = Instant::now();
    let report = reconcile.run(&ws.id).await;
    let elapsed = at.elapsed().as_millis();

    // Every file landed, or this measures a shorter run than it claims. The commonest way a budget
    // test lies is by measuring a loop that gave up early.
    assert_eq!(report.files.len(), FILES);
    let landed = report
        .files
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::FastForwarded | Outcome::Merged))
        .count();
    assert_eq!(landed, FILES, "every file must have reconciled: {report:?}");
    assert!(
        store.pending_edits(&ws.id).expect("read").is_empty(),
        "and every row is gone, which is what a confirmed write means"
    );

    println!(
        "SC-011 reconciling {FILES} files ({} fast-forwards, {} merges): {elapsed} ms of the \
         {BUDGET_MS} ms budget",
        FILES / 2,
        FILES / 2
    );
    assert!(
        elapsed < BUDGET_MS,
        "{elapsed} ms exceeded the {BUDGET_MS} ms budget"
    );
}
