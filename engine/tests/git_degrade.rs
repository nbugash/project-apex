//! A workspace without git is a working workspace.
//!
//! Two different absences reach the client as the **same** empty status: a directory that is
//! not a repository, and a host where git cannot be run at all (FR-027, FR-028). Both are
//! asserted here rather than one, because the engine deliberately keeps them apart internally
//! — `GitFailure::NotARepository` and `GitFailure::GitUnavailable` are distinct so diagnostics
//! can tell an ordinary directory from a misconfigured host — and only a test says whether the
//! collapse into one client-visible answer actually happens.

mod common;

use apex_engine::adapters::outbound::git_cli::GitCli;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::git::{Git, GitFailure};
use apex_engine::application::use_cases::git_status::{status_or_nothing, GitService};
use apex_engine::domain::path::ResolvedPath;
use apex_protocol::wire::BranchPosition;
use common::repo::Repo;
use std::sync::Arc;

fn resolved(root: &std::path::Path) -> ResolvedPath {
    let fs = StdFileSystem;
    let canonical = ResolvedPath::canonical_root(root, &fs).expect("canonical root");
    ResolvedPath::resolve(&canonical, ".", &fs).expect("resolve")
}

/// A git that is not on this host. The name is the whole fixture: `Command::spawn` fails with
/// `NotFound`, which is exactly what a host without git produces.
fn absent_git() -> GitCli {
    GitCli::new("apex-no-such-git-binary")
}

#[test]
fn a_directory_that_is_not_a_repository_reports_not_a_repository() {
    // The distinction the engine keeps. Asserted at the port, because above it the answer is
    // deliberately indistinguishable from the next test's.
    let (dir, _keep) = common::repo::not_a_repo();
    assert_eq!(
        GitCli::default().status(&resolved(&dir)),
        Err(GitFailure::NotARepository)
    );
}

#[test]
fn a_host_without_git_reports_git_unavailable() {
    let repo = Repo::new();
    assert_eq!(
        absent_git().status(&resolved(&repo.root)),
        Err(GitFailure::GitUnavailable)
    );
}

#[test]
fn both_absences_are_the_same_empty_answer_one_level_up() {
    // FR-027 and FR-028. The client's behaviour is identical in both cases, so the collapse
    // happens here and the wire carries one shape rather than two.
    let (dir, _keep) = common::repo::not_a_repo();
    let no_repo = status_or_nothing(&GitCli::default(), &resolved(&dir)).expect("an absence");

    let repo = Repo::new();
    let no_git = status_or_nothing(&absent_git(), &resolved(&repo.root)).expect("an absence");

    assert_eq!(
        no_repo, no_git,
        "the two absences must be indistinguishable"
    );
    assert_eq!(no_repo.branch, BranchPosition::None);
    assert!(no_repo.changes.is_empty());
}

#[test]
fn an_absence_is_a_success_and_not_an_error() {
    // The assertion that matters for the workspace as a whole: an error here would fail the
    // workspace, and a developer who opened a plain directory would see a failure where the
    // correct answer is "there is no git state".
    let (dir, _keep) = common::repo::not_a_repo();
    assert!(status_or_nothing(&GitCli::default(), &resolved(&dir)).is_ok());
}

#[test]
fn the_service_serves_an_empty_first_page_for_a_workspace_without_git() {
    // End to end through the layer dispatch actually calls. `status_or_nothing` being right is
    // not the same as the pager being asked at all, and a service that propagated the failure
    // would pass every assertion above.
    let (dir, _keep) = common::repo::not_a_repo();
    let service = GitService::new(Arc::new(GitCli::default()));
    let page = service
        .refresh("w1", &resolved(&dir), 1_000)
        .expect("an absence is a successful empty status");
    assert_eq!(page.current_branch, BranchPosition::None);
    assert!(page.changes.is_empty());
    assert!(
        page.next_cursor.is_none(),
        "an empty status is a final page, so a client knows not to ask again"
    );
}

#[test]
fn a_diff_degrades_the_same_way_a_status_does() {
    // The same rule applied to the other request. A client that could read a status but got an
    // error for a diff would show the workspace as partly broken.
    let (dir, _keep) = common::repo::not_a_repo();
    let service = GitService::new(Arc::new(GitCli::default()));
    let diff = service
        .file_diff(&resolved(&dir), "anything.rs")
        .expect("an absence is an empty diff");
    assert!(diff.added.is_empty() && diff.modified.is_empty() && diff.deleted.is_empty());
}

#[test]
fn a_genuine_failure_is_still_a_failure() {
    // The other side of the rule, and the reason `is_absence` exists rather than a blanket
    // `unwrap_or_default`. Degrading *everything* to empty would hide a real problem behind a
    // clean-looking repository, which is the most misleading answer this feature can give.
    assert!(!GitFailure::Failed("exploded".into()).is_absence());
    assert!(GitFailure::NotARepository.is_absence());
    assert!(GitFailure::GitUnavailable.is_absence());
}

// ---- A workspace inside somebody else's repository ----

#[test]
fn a_subdirectory_workspace_reports_only_its_own_subtree() {
    // FR-003a. **The monorepo case, supported rather than refused.** `rev-parse` walks upward, so
    // without scoping this workspace receives the enclosing repository's changes -- on a
    // monorepo, most of somebody else's work -- under paths relative to the *repository* root,
    // which name nothing that exists here.
    //
    // The first version of this feature refused the case outright for exactly that reason.
    // Opening a subdirectory of a large repository is an ordinary way to work, so the status
    // is scoped with `-- .` and every path re-rooted instead.
    let repo = Repo::new();
    repo.write("src/mine.txt", "changed here\n");
    repo.write("kept.txt", "changed elsewhere\n");
    repo.write("src/untracked.txt", "new here\n");

    let inside = repo.root.join("src");
    let status = GitCli::default()
        .status(&resolved(&inside))
        .expect("a subdirectory of a repository has git state");

    let paths: Vec<&str> = status.changes.iter().map(|c| c.path.as_str()).collect();
    assert!(
        paths.contains(&"/mine.txt"),
        "a change inside the workspace must be reported, re-rooted: {paths:?}"
    );
    assert!(
        paths.contains(&"/untracked.txt"),
        "an untracked file inside the workspace must be reported: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains("kept.txt")),
        "a change outside the workspace leaked in: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("/src/")),
        "paths are relative to the workspace, not the repository: {paths:?}"
    );
}

#[test]
fn a_subdirectory_workspace_reports_the_repository_s_branch() {
    // The branch is the repository's and needs no re-rooting: a subtree does not have one of
    // its own, and the developer working in it is on that branch.
    let repo = Repo::new();
    repo.run(&["checkout", "-q", "-b", "feature/thing"]);
    let inside = repo.root.join("src");
    assert_eq!(
        GitCli::default()
            .status(&resolved(&inside))
            .expect("status")
            .branch,
        BranchPosition::Branch("feature/thing".into())
    );
}

#[test]
fn a_subdirectory_workspace_diffs_its_own_files() {
    // No prefix arithmetic on this path: git runs with the workspace root as its working
    // directory and a pathspec is resolved against that.
    let repo = Repo::new();
    repo.write("src/a.txt", "one\nCHANGED\nthree\nfour\n");
    let inside = repo.root.join("src");
    let diff = GitCli::default()
        .file_diff(&resolved(&inside), "a.txt")
        .expect("diff");
    assert!(
        !diff.modified.is_empty(),
        "a file inside a subdirectory workspace reported no change: {diff:?}"
    );
}

#[test]
fn the_repository_root_itself_still_works() {
    // The other side of the rule. An equality check that was accidentally never true would
    // turn every repository into a non-repository, and every test above would still pass.
    let repo = Repo::new();
    repo.write("src/a.txt", "changed\n");
    let service = GitService::new(Arc::new(GitCli::default()));
    let page = service
        .refresh("w1", &resolved(&repo.root), 1_000)
        .expect("status");
    assert_eq!(page.current_branch, BranchPosition::Branch("main".into()));
    assert!(
        page.changes.iter().any(|c| c.path == "/src/a.txt"),
        "the repository root must still report its own changes: {:?}",
        page.changes
    );
}

#[test]
fn a_wholly_untracked_subdirectory_workspace_still_reports_its_files() {
    // FR-003b. **The case that broke the first version of subdirectory support.** git collapses a
    // directory with nothing tracked in it to a single `? sub/` entry -- which, for a
    // workspace *on* `sub/`, strips to the empty string and disappears. A developer who made a
    // new directory inside a repository and opened it would see no markers at all, on a
    // workspace where every single file is new.
    let repo = Repo::new();
    std::fs::create_dir_all(repo.root.join("fresh/deep")).unwrap();
    repo.write("fresh/a.txt", "a\n");
    repo.write("fresh/deep/b.txt", "b\n");

    let inside = repo.root.join("fresh");
    let status = GitCli::default()
        .status(&resolved(&inside))
        .expect("status");
    let paths: Vec<&str> = status.changes.iter().map(|c| c.path.as_str()).collect();

    assert!(
        paths.contains(&"/a.txt") && paths.contains(&"/deep/b.txt"),
        "an untracked subdirectory workspace reported {paths:?}"
    );
}

#[test]
fn the_repository_root_still_collapses_untracked_directories() {
    // The other half of the same decision. Expanding at the root is unbounded -- one entry per
    // file under every untracked directory -- and a collapsed `? build/` is what the tree
    // wants anyway.
    let repo = Repo::new();
    std::fs::create_dir_all(repo.root.join("build/nested")).unwrap();
    repo.write("build/one.o", "x\n");
    repo.write("build/nested/two.o", "y\n");

    let status = GitCli::default()
        .status(&resolved(&repo.root))
        .expect("status");
    let paths: Vec<&str> = status.changes.iter().map(|c| c.path.as_str()).collect();
    assert!(
        paths.contains(&"/build"),
        "the root workspace should report the directory, not its contents: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains("one.o")),
        "the root workspace expanded an untracked directory: {paths:?}"
    );
}
