//! Git state with no connection.
//!
//! **A cleared projection is the one answer that must never be given.** It says nothing has
//! changed, which is a positive claim about the repository — and offline is precisely when the
//! client does not know. Showing the last state it did know, on the same terms as any other
//! content it cannot confirm, is the only honest option (FR-029).

mod common;

use apex_protocol::wire::{BranchPosition, GitChange, GitStatusKind, GitStatusUpdate};
use apex_shell::application::ports::git_provider::GitProvider;
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::ports::workspace_provider::{ProviderError, ProviderResult};
use apex_shell::application::use_cases::apply_git_status::{ApplyGitStatus, ApplyOutcome};
use apex_shell::domain::workspace::{Location, Workspace, WorkspaceId};
use async_trait::async_trait;
use common::fake_cache::InMemoryCache;
use std::sync::Arc;

/// A provider with nothing on the other end. Every call is `Offline`, which is what the
/// transport reports once the connection is gone.
struct Disconnected;

#[async_trait]
impl GitProvider for Disconnected {
    async fn status(
        &self,
        _: &WorkspaceId,
        _: Option<&str>,
    ) -> ProviderResult<apex_protocol::wire::GitStatusResult> {
        Err(ProviderError::Offline)
    }
    async fn file_diff(
        &self,
        _: &WorkspaceId,
        _: &str,
    ) -> ProviderResult<apex_protocol::wire::GitDiffResult> {
        Err(ProviderError::Offline)
    }
    async fn recently_changed(&self, _: &WorkspaceId) -> ProviderResult<Vec<String>> {
        Err(ProviderError::Offline)
    }
}

fn ws() -> WorkspaceId {
    WorkspaceId("w1".into())
}

fn registered() -> Arc<InMemoryCache> {
    let cache = Arc::new(InMemoryCache::new());
    cache
        .register(
            &Workspace {
                id: ws(),
                name: "repo".into(),
                location: Location::Local {
                    base: "/repo".into(),
                },
                last_opened_at: 0,
            },
            0,
        )
        .expect("register");
    cache
}

fn change(path: &str, status: GitStatusKind) -> GitChange {
    GitChange {
        path: path.into(),
        status,
    }
}

fn update(changes: Vec<GitChange>, next: Option<&str>) -> GitStatusUpdate {
    GitStatusUpdate {
        workspace_id: apex_protocol::wire::WorkspaceId("w1".into()),
        current_branch: BranchPosition::Branch("main".into()),
        changes,
        next_cursor: next.map(str::to_owned),
    }
}

#[tokio::test]
async fn the_last_applied_state_is_still_readable_with_no_connection() {
    let cache = registered();
    let online = Arc::new(common::fake_git::FakeGit::new());
    ApplyGitStatus::new(online, cache.clone())
        .apply(update(
            vec![
                change("/src/a.rs", GitStatusKind::Modified),
                change("/src/b.rs", GitStatusKind::Conflict),
            ],
            None,
        ))
        .await;

    // The connection goes. Reading does not consult it -- there is no provider on this path at
    // all, which is why an outage costs nothing and times out never.
    let read = cache.git_status(&ws()).expect("read");
    assert_eq!(read.changes.len(), 2);
    assert_eq!(read.branch, BranchPosition::Branch("main".into()));
}

#[tokio::test]
async fn an_update_that_cannot_be_completed_offline_leaves_the_state_alone() {
    let cache = registered();
    ApplyGitStatus::new(Arc::new(common::fake_git::FakeGit::new()), cache.clone())
        .apply(update(
            vec![change("/known.rs", GitStatusKind::Staged)],
            None,
        ))
        .await;
    let before = cache.git_status(&ws()).expect("read");

    // A first page arrives and then the connection drops mid-pull.
    let outcome = ApplyGitStatus::new(Arc::new(Disconnected), cache.clone())
        .apply(update(
            vec![change("/other.rs", GitStatusKind::Modified)],
            Some("c1"),
        ))
        .await;

    assert!(matches!(outcome, ApplyOutcome::Discarded(_)));
    assert_eq!(
        cache.git_status(&ws()).expect("read"),
        before,
        "an outage mid-pull must not change what is shown"
    );
}

#[tokio::test]
async fn an_outage_never_clears_the_projection() {
    // Stated as its own assertion because it is the failure mode, not a corollary: an
    // implementation that cleared on disconnection would pass a test asserting only that the
    // update was discarded.
    let cache = registered();
    ApplyGitStatus::new(Arc::new(common::fake_git::FakeGit::new()), cache.clone())
        .apply(update(vec![change("/a.rs", GitStatusKind::Modified)], None))
        .await;

    for _ in 0..3 {
        ApplyGitStatus::new(Arc::new(Disconnected), cache.clone())
            .apply(update(Vec::new(), Some("c1")))
            .await;
    }

    assert_eq!(
        cache.git_status(&ws()).expect("read").changes.len(),
        1,
        "repeated failures must not erode the state"
    );
}

#[tokio::test]
async fn a_diff_offline_is_a_refusal_and_not_an_empty_diff() {
    // An empty diff means "this file has no changes", which offline is exactly not known. The
    // caller must be able to tell "nothing differs" from "I cannot say".
    let result = Disconnected.file_diff(&ws(), "/src/a.rs").await;
    assert!(matches!(result, Err(ProviderError::Offline)));
}
