//! Applying git status, which is where the one interesting mistake lives.
//!
//! A status too large for a frame arrives as a notification carrying the **first page** and a
//! cursor. An implementation that applied that notification as it stood would mark every path
//! beyond the first page as unchanged — and would pass every test written against a single
//! message, on every repository small enough to fit in one. So the assertions here are mostly
//! about what must **not** happen yet (A-GITPAGE, FR-009, FR-009a).

mod common;

use apex_protocol::wire::{
    BranchPosition, GitChange, GitStatusKind, GitStatusResult, GitStatusUpdate,
};
use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::application::use_cases::apply_git_status::{
    ApplyGitStatus, ApplyOutcome, Discarded,
};
use apex_shell::domain::workspace::{Location, Workspace, WorkspaceId};
use common::fake_cache::InMemoryCache;
use common::fake_git::FakeGit;
use std::sync::Arc;

fn change(path: &str, status: GitStatusKind) -> GitChange {
    GitChange {
        path: path.into(),
        status,
    }
}

fn changes(prefix: &str, n: usize) -> Vec<GitChange> {
    (0..n)
        .map(|i| change(&format!("/{prefix}{i:04}.rs"), GitStatusKind::Modified))
        .collect()
}

fn update(changes: Vec<GitChange>, next: Option<&str>) -> GitStatusUpdate {
    GitStatusUpdate {
        workspace_id: apex_protocol::wire::WorkspaceId("w1".into()),
        current_branch: BranchPosition::Branch("main".into()),
        changes,
        next_cursor: next.map(str::to_owned),
    }
}

fn page(changes: Vec<GitChange>, next: Option<&str>) -> GitStatusResult {
    GitStatusResult {
        current_branch: BranchPosition::Branch("main".into()),
        changes,
        next_cursor: next.map(str::to_owned),
    }
}

/// A cache with `w1` registered, as a client that has the workspace open would have.
fn registered() -> Arc<InMemoryCache> {
    let cache = Arc::new(InMemoryCache::new());
    cache
        .register(
            &Workspace {
                id: WorkspaceId("w1".into()),
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

fn ws() -> WorkspaceId {
    WorkspaceId("w1".into())
}

#[tokio::test]
async fn a_single_page_update_is_applied_whole() {
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let outcome = apply.apply(update(changes("a", 3), None)).await;
    assert!(matches!(
        outcome,
        ApplyOutcome::Applied {
            paths: 3,
            pages: 1,
            ..
        }
    ));
    assert_eq!(cache.git_status(&ws()).expect("read").changes.len(), 3);
    assert!(
        git.asked_for().is_empty(),
        "a complete update must issue no request at all"
    );
}

#[tokio::test]
async fn a_first_page_with_a_cursor_is_not_applied_until_the_last_page_arrives() {
    // FR-009a, and the single most likely way to get this feature wrong. The notification alone
    // is a *prefix* of the answer, and applying a prefix says the rest is unchanged.
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    // The second page never arrives: asking for `c1` is refused.
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let outcome = apply.apply(update(changes("a", 1_000), Some("c1"))).await;

    assert!(
        matches!(outcome, ApplyOutcome::Discarded(Discarded::PullFailed(_))),
        "an interrupted pull must be discarded, got {outcome:?}"
    );
    assert!(
        cache.git_status(&ws()).expect("read").changes.is_empty(),
        "the first page must not have been written on its own"
    );
}

#[tokio::test]
async fn every_page_of_a_chain_arrives_before_anything_is_applied() {
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    git.page("c1", page(changes("b", 1_000), Some("c2")));
    git.page("c2", page(changes("c", 500), None));
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let outcome = apply.apply(update(changes("a", 1_000), Some("c1"))).await;
    assert!(
        matches!(
            outcome,
            ApplyOutcome::Applied {
                paths: 2_500,
                pages: 3,
                ..
            }
        ),
        "got {outcome:?}"
    );
    assert_eq!(cache.git_status(&ws()).expect("read").changes.len(), 2_500);
    assert_eq!(git.asked_for(), vec!["c1".to_string(), "c2".to_string()]);
}

#[tokio::test]
async fn an_interrupted_pull_leaves_the_previous_state_exactly_as_it_was() {
    // Not merely "leaves something": the *previous* answer, unchanged. A discard that cleared
    // would tell the developer nothing had changed, which is the one thing it must not say.
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    apply
        .apply(update(
            vec![change("/kept.rs", GitStatusKind::Conflict)],
            None,
        ))
        .await;
    let before = cache.git_status(&ws()).expect("read");

    let outcome = apply.apply(update(changes("new", 10), Some("gone"))).await;
    assert!(matches!(outcome, ApplyOutcome::Discarded(_)));

    let after = cache.git_status(&ws()).expect("read");
    assert_eq!(before, after, "a discarded pull changed the stored state");
    assert_eq!(after.changes[0].status, GitStatusKind::Conflict);
}

#[tokio::test]
async fn a_failing_commit_discards_rather_than_half_applies() {
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());
    apply.apply(update(changes("old", 2), None)).await;

    cache.fail_writes("disk full");
    let outcome = apply.apply(update(changes("new", 5), None)).await;
    assert!(matches!(
        outcome,
        ApplyOutcome::Discarded(Discarded::CommitFailed(_))
    ));
    cache.allow_writes();
    assert_eq!(
        cache.git_status(&ws()).expect("read").changes.len(),
        2,
        "the previous answer must survive a failed replacement"
    );
}

#[tokio::test]
async fn an_update_naming_an_unknown_workspace_is_discarded() {
    // FR-012. Nothing about an unregistered workspace can be stored, because there is nothing
    // for it to belong to -- the store enforces this with a foreign key.
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let mut u = update(changes("a", 2), None);
    u.workspace_id = apex_protocol::wire::WorkspaceId("never-opened".into());
    let outcome = apply.apply(u).await;

    assert!(matches!(
        outcome,
        ApplyOutcome::Discarded(Discarded::CommitFailed(_))
    ));
    assert!(cache.git_status(&ws()).expect("read").changes.is_empty());
}

#[tokio::test]
async fn one_workspace_s_update_never_alters_another_s_rows() {
    // FR-011. Wholesale replacement is scoped by workspace; a replace that forgot the scope
    // would clear every other workspace on every update, which is invisible with one open.
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
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let mut for_w2 = update(changes("z", 4), None);
    for_w2.workspace_id = apex_protocol::wire::WorkspaceId("w2".into());
    apply.apply(for_w2).await;
    apply.apply(update(changes("a", 7), None)).await;

    assert_eq!(cache.git_status(&ws()).expect("read").changes.len(), 7);
    assert_eq!(
        cache
            .git_status(&WorkspaceId("w2".into()))
            .expect("read")
            .changes
            .len(),
        4,
        "applying w1's update changed w2's rows"
    );
}

#[tokio::test]
async fn a_path_escaping_the_workspace_is_refused_by_the_client_too() {
    // The cross-boundary obligation: the engine contains paths before emitting and the client
    // contains them again. Not redundant -- this catches a bug in our own engine, which is the
    // failure a one-sided boundary cannot catch at all (Principle VI).
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let outcome = apply
        .apply(update(
            vec![
                change("/fine.rs", GitStatusKind::Modified),
                change("/../escaped.rs", GitStatusKind::Modified),
                change("../../etc/passwd", GitStatusKind::Modified),
            ],
            None,
        ))
        .await;

    let ApplyOutcome::Applied { paths, refused, .. } = outcome else {
        panic!("expected an application, got {outcome:?}");
    };
    assert_eq!(refused, 2, "both escaping paths must be refused");
    assert_eq!(paths, 1);
    assert_eq!(cache.git_status(&ws()).expect("read").changes.len(), 1);
}

#[tokio::test]
async fn an_endless_cursor_chain_is_abandoned_rather_than_followed_forever() {
    // Our own engine is untrusted input. A cursor that returns itself is a bug this client
    // cannot fix and must not hang on.
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    git.page("loop", page(changes("x", 1), Some("loop")));
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let outcome = apply.apply(update(changes("a", 1), Some("loop"))).await;
    assert_eq!(outcome, ApplyOutcome::Discarded(Discarded::TooManyPages));
    assert!(cache.git_status(&ws()).expect("read").changes.is_empty());
}

#[tokio::test]
async fn an_untracked_file_in_a_folder_the_tree_has_never_listed_is_stored() {
    // FR-009b, SC-014. **The test that justifies keying by path.** The tree-keyed design F003
    // shipped passes every other assertion in this file and fails only this one: there is no
    // row for `/deep/never/listed/new.rs` to hang from, because nothing has ever listed it.
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    apply
        .apply(update(
            vec![change(
                "/deep/never/listed/new.rs",
                GitStatusKind::Untracked,
            )],
            None,
        ))
        .await;

    let stored = cache.git_status(&ws()).expect("read");
    assert_eq!(stored.changes.len(), 1);
    assert_eq!(stored.changes[0].path, "/deep/never/listed/new.rs");
    assert_eq!(stored.changes[0].status, GitStatusKind::Untracked);
}

#[tokio::test]
async fn applying_updates_leaves_cached_files_and_their_hashes_untouched() {
    // FR-010, SC-006, §5.3. Git status says what differs from the repository, which is not a
    // statement about whether a cached copy still matches the host. Asserted by counting,
    // because the failure is silent: content quietly re-fetched costs a round trip per file and
    // shows up as "the client feels slow" rather than as anything one could attribute.
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let before = cache.content_fingerprint();
    for i in 0..5 {
        apply
            .apply(update(changes(&format!("round{i}"), 50), None))
            .await;
    }
    assert_eq!(
        cache.content_fingerprint(),
        before,
        "applying git status changed cached content or its hashes"
    );
}

#[tokio::test]
async fn the_branch_travels_with_the_changes_it_describes() {
    let cache = registered();
    let git = Arc::new(FakeGit::new());
    git.page("c1", {
        let mut p = page(changes("b", 2), None);
        p.current_branch = BranchPosition::Branch("main".into());
        p
    });
    let apply = ApplyGitStatus::new(git.clone(), cache.clone());

    let mut u = update(changes("a", 2), Some("c1"));
    u.current_branch = BranchPosition::Detached("abc1234".into());
    apply.apply(u).await;

    assert_eq!(
        cache.git_status(&ws()).expect("read").branch,
        BranchPosition::Detached("abc1234".into()),
        "the branch comes from the update that began the pull, with the changes it describes"
    );
}
