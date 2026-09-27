//! What a bulk invalidation does to git state, and what it must not touch.
//!
//! A branch switch arrives as `workspace/invalidateAll` and is followed by a fresh status. The
//! interesting question is what the client shows **in between**, and the answer is not
//! obvious: the marks it holds describe the branch the developer has just left, and a tree that
//! went on showing them would be confidently wrong about every file (FR-025, FR-026, SC-011).
//!
//! It must also **not** refetch anything. A burst of listings at the moment a link has just
//! proved unreliable is the worst time to issue one (FR-026a), and cached content stays valid
//! or not on its own hash terms -- invalidation of the tree is not invalidation of content
//! (§5.3).

mod common;

use apex_protocol::wire::{BranchPosition, GitChange, GitStatusKind, GitStatusUpdate};
use apex_shell::application::ports::workspace_cache::{GitProjection, WorkspaceCache};
use apex_shell::application::use_cases::apply_git_status::ApplyGitStatus;
use apex_shell::domain::workspace::{Location, Workspace, WorkspaceId};
use common::fake_cache::InMemoryCache;
use common::fake_git::FakeGit;
use std::sync::Arc;

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

fn on_branch(name: &str, changes: Vec<GitChange>) -> GitStatusUpdate {
    GitStatusUpdate {
        workspace_id: apex_protocol::wire::WorkspaceId("w1".into()),
        current_branch: BranchPosition::Branch(name.into()),
        changes,
        next_cursor: None,
    }
}

#[tokio::test]
async fn invalidating_clears_the_marks_from_the_branch_being_left() {
    // SC-011. The marks describe a branch the developer is no longer on, and every one of them
    // is now a claim about a file that may not even exist here.
    let cache = registered();
    let apply = ApplyGitStatus::new(Arc::new(FakeGit::new()), cache.clone());
    apply
        .apply(on_branch(
            "old",
            vec![
                change("/only-on-old.rs", GitStatusKind::Modified),
                change("/shared.rs", GitStatusKind::Staged),
            ],
        ))
        .await;
    assert_eq!(cache.git_status(&ws()).expect("read").changes.len(), 2);

    apply.invalidate(&ws());

    let after = cache.git_status(&ws()).expect("read");
    assert!(
        after.changes.is_empty(),
        "marks from the previous branch survived an invalidation: {:?}",
        after.changes
    );
    assert_eq!(
        after.branch,
        BranchPosition::None,
        "the branch left behind must not be named either"
    );
}

#[tokio::test]
async fn invalidating_does_not_refetch_anything() {
    // FR-026a. A burst of requests at the moment a switch has just churned the whole tree is
    // the worst time to issue one, and the status that follows is being pushed anyway.
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());
    apply
        .apply(on_branch(
            "old",
            vec![change("/a.rs", GitStatusKind::Modified)],
        ))
        .await;

    apply.invalidate(&ws());

    assert!(
        git.asked_for().is_empty(),
        "invalidation issued requests: {:?}",
        git.asked_for()
    );
}

#[tokio::test]
async fn invalidating_leaves_cached_content_and_its_hashes_alone() {
    // §5.3. A branch switch changes which files differ from the repository; it says nothing
    // about whether a cached copy still matches the host, which is a hash comparison.
    let cache = registered();
    let apply = ApplyGitStatus::new(Arc::new(FakeGit::new()), cache.clone());
    apply
        .apply(on_branch(
            "old",
            vec![change("/a.rs", GitStatusKind::Modified)],
        ))
        .await;

    let before = cache.content_fingerprint();
    apply.invalidate(&ws());
    assert_eq!(
        cache.content_fingerprint(),
        before,
        "invalidation touched cached content"
    );
}

#[tokio::test]
async fn the_next_status_after_an_invalidation_is_the_new_branch_s() {
    let cache = registered();
    let apply = ApplyGitStatus::new(Arc::new(FakeGit::new()), cache.clone());
    apply
        .apply(on_branch(
            "old",
            vec![change("/gone.rs", GitStatusKind::Modified)],
        ))
        .await;
    apply.invalidate(&ws());
    apply
        .apply(on_branch(
            "new",
            vec![change("/here.rs", GitStatusKind::Untracked)],
        ))
        .await;

    let after = cache.git_status(&ws()).expect("read");
    assert_eq!(after.branch, BranchPosition::Branch("new".into()));
    assert_eq!(after.changes.len(), 1);
    assert_eq!(after.changes[0].path, "/here.rs");
}

#[tokio::test]
async fn invalidating_one_workspace_leaves_another_alone() {
    let cache = registered();
    cache
        .register(
            &Workspace {
                id: WorkspaceId("w2".into()),
                name: "other".into(),
                location: Location::Local {
                    base: "/other".into(),
                },
                last_opened_at: 0,
            },
            0,
        )
        .expect("register w2");
    let apply = ApplyGitStatus::new(Arc::new(FakeGit::new()), cache.clone());

    let mut for_w2 = on_branch("kept", vec![change("/b.rs", GitStatusKind::Modified)]);
    for_w2.workspace_id = apex_protocol::wire::WorkspaceId("w2".into());
    apply.apply(for_w2).await;
    apply
        .apply(on_branch(
            "old",
            vec![change("/a.rs", GitStatusKind::Modified)],
        ))
        .await;

    apply.invalidate(&ws());

    assert!(cache.git_status(&ws()).expect("read").changes.is_empty());
    assert_eq!(
        cache
            .git_status(&WorkspaceId("w2".into()))
            .expect("read")
            .changes
            .len(),
        1,
        "invalidating one workspace cleared another's marks"
    );
}

#[tokio::test]
async fn invalidating_an_unknown_workspace_is_harmless() {
    let cache = registered();
    let apply = ApplyGitStatus::new(Arc::new(FakeGit::new()), cache.clone());
    apply.invalidate(&WorkspaceId("never-opened".into()));
    // Deliberately no assertion on the unknown workspace: the claim is that the call does not
    // panic and does not disturb the one that exists.
    assert!(cache.git_status(&ws()).expect("read").changes.is_empty());
}

#[tokio::test]
async fn a_projection_cleared_by_invalidation_is_not_the_same_as_one_never_written() {
    // The distinction matters for FR-029: offline, the client keeps what it last knew. An
    // invalidation is a positive statement that what it knew is now wrong, which is the one
    // case where clearing is right.
    let cache = registered();
    let apply = ApplyGitStatus::new(Arc::new(FakeGit::new()), cache.clone());
    apply
        .apply(on_branch(
            "old",
            vec![change("/a.rs", GitStatusKind::Modified)],
        ))
        .await;
    apply.invalidate(&ws());
    let cleared = cache.git_status(&ws()).expect("read");
    assert_eq!(cleared, GitProjection::default());
}
