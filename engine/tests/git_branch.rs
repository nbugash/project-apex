//! Where the repository is, which is not always a branch.
//!
//! **The header is not a name.** `--porcelain=v2 --branch` writes `# branch.head (detached)`
//! when HEAD is detached, in the position a branch name occupies — so the obvious reading,
//! "take the rest of the line", produces a repository apparently sitting on a branch called
//! `(detached)`. A developer mid-rebase would be told they are on a branch they cannot push,
//! by an indicator that looks entirely normal (research.md, FR-018, FR-019).

mod common;

use apex_engine::adapters::outbound::git_cli::GitCli;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::git::Git;
use apex_engine::domain::path::ResolvedPath;
use apex_protocol::wire::BranchPosition;
use common::repo::Repo;

fn resolved(root: &std::path::Path) -> ResolvedPath {
    let fs = StdFileSystem;
    let canonical = ResolvedPath::canonical_root(root, &fs).expect("canonical root");
    ResolvedPath::resolve(&canonical, ".", &fs).expect("resolve")
}

fn branch_of(repo: &Repo) -> BranchPosition {
    GitCli::default()
        .status(&resolved(&repo.root))
        .expect("status")
        .branch
}

#[test]
fn a_named_branch_is_reported_by_name() {
    let repo = Repo::new();
    repo.run(&["checkout", "-q", "-b", "feature/thing"]);
    assert_eq!(
        branch_of(&repo),
        BranchPosition::Branch("feature/thing".into()),
        "a branch with a slash in it is still one name"
    );
}

#[test]
fn a_detached_head_is_the_detached_case_and_not_a_branch_named_detached() {
    // The whole reason `BranchPosition` has three cases rather than an optional string.
    let repo = Repo::new();
    repo.detach();
    match branch_of(&repo) {
        BranchPosition::Detached(commit) => {
            assert!(!commit.is_empty(), "a detached head must name its commit");
            assert!(
                !commit.contains("detached"),
                "the commit is an object id, not the header's text: {commit}"
            );
        }
        other => panic!("a detached head reported as {other:?}"),
    }
}

#[test]
fn a_repository_with_no_commits_still_reports_its_branch_name() {
    // An unborn branch has a name and no commit. `# branch.oid (initial)` is what git writes,
    // and a parser that resolved the oid first would fail on a repository a developer has just
    // created -- the first thing many people do.
    let repo = Repo::empty();
    assert_eq!(
        branch_of(&repo),
        BranchPosition::Branch("main".into()),
        "an unborn branch shows a name rather than nothing"
    );
}

#[test]
fn a_directory_that_is_not_a_repository_has_no_branch() {
    // FR-019: nothing, rather than an empty name. A blank label on the status bar reads as a
    // rendering fault; an absent one reads as "there is no repository here", which is true.
    let (dir, _keep) = common::repo::not_a_repo();
    let failure = GitCli::default().status(&resolved(&dir));
    assert!(failure.is_err(), "a plain directory is not a repository");
}

#[test]
fn the_branch_follows_a_switch() {
    let repo = Repo::new();
    assert_eq!(branch_of(&repo), BranchPosition::Branch("main".into()));
    repo.run(&["checkout", "-q", "-b", "elsewhere"]);
    assert_eq!(branch_of(&repo), BranchPosition::Branch("elsewhere".into()));
}

#[test]
fn a_linked_worktree_reports_its_own_branch() {
    // Each worktree has its own HEAD. A reader that resolved the main repository's would name
    // the wrong branch on every worktree, while working perfectly on the common case.
    let repo = Repo::new();
    let wt = repo.add_worktree("wt");
    let main = branch_of(&repo);
    let theirs = GitCli::default()
        .status(&resolved(&wt))
        .expect("status")
        .branch;
    assert_ne!(main, theirs, "a worktree is on a different branch");
    assert!(matches!(theirs, BranchPosition::Branch(_)));
}
