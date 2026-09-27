//! Git state survives the client being closed.
//!
//! **Asserted on a reopened store, not a live one.** FR-013 says the marks are there when the
//! workspace is opened again, before any refresh has arrived — and an in-memory projection
//! beside the database satisfies every assertion that keeps one handle open. Only closing the
//! store and opening it again distinguishes "persisted" from "remembered".

use apex_protocol::wire::{BranchPosition, GitChange, GitStatusKind};
use apex_shell::adapters::outbound::sqlite::schema::CURRENT_VERSION;
use apex_shell::adapters::outbound::sqlite::SqliteWorkspaceCache;
use apex_shell::application::ports::workspace_cache::{GitProjection, WorkspaceCache};
use apex_shell::domain::workspace::{Location, Workspace, WorkspaceId};

fn ws() -> WorkspaceId {
    WorkspaceId("w1".into())
}

fn workspace() -> Workspace {
    Workspace {
        id: ws(),
        name: "repo".into(),
        location: Location::Local {
            base: "/repo".into(),
        },
        last_opened_at: 0,
    }
}

/// A store on disk, at the current schema, with `w1` registered.
fn opened(path: &std::path::Path) -> SqliteWorkspaceCache {
    let store = SqliteWorkspaceCache::open(path).expect("open");
    store
        .migrate_to(CURRENT_VERSION, &mut |_| {})
        .expect("migrate");
    store.register(&workspace(), 0).expect("register");
    store
}

fn change(path: &str, status: GitStatusKind) -> GitChange {
    GitChange {
        path: path.into(),
        status,
    }
}

#[test]
fn git_state_written_by_one_session_is_there_for_the_next() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cache.db");

    {
        let store = opened(&path);
        store
            .replace_git_status(
                &ws(),
                &GitProjection {
                    branch: BranchPosition::Branch("feature/thing".into()),
                    changes: vec![
                        change("/src/a.rs", GitStatusKind::Modified),
                        change("/src/b.rs", GitStatusKind::Staged),
                        change("/new.rs", GitStatusKind::Untracked),
                    ],
                },
            )
            .expect("replace");
    } // the session ends; the handle is dropped

    // A new session, and nothing has refreshed.
    let store = opened(&path);
    let read = store.git_status(&ws()).expect("read");
    assert_eq!(
        read.branch,
        BranchPosition::Branch("feature/thing".into()),
        "the branch must survive a restart"
    );
    assert_eq!(read.changes.len(), 3);
    assert_eq!(
        read.changes
            .iter()
            .find(|c| c.path == "/src/b.rs")
            .map(|c| c.status),
        Some(GitStatusKind::Staged),
        "each path must keep the state it was stored with"
    );
}

#[test]
fn a_replacement_removes_what_is_no_longer_reported() {
    // The half of "wholesale" that a delta would get wrong. A path that has stopped differing
    // is reported by its **absence** from the new answer, so anything less than a full replace
    // leaves it marked until the workspace is closed.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cache.db");
    let store = opened(&path);

    store
        .replace_git_status(
            &ws(),
            &GitProjection {
                branch: BranchPosition::Branch("main".into()),
                changes: vec![
                    change("/gone.rs", GitStatusKind::Modified),
                    change("/stays.rs", GitStatusKind::Modified),
                ],
            },
        )
        .expect("first");
    store
        .replace_git_status(
            &ws(),
            &GitProjection {
                branch: BranchPosition::Branch("main".into()),
                changes: vec![change("/stays.rs", GitStatusKind::Staged)],
            },
        )
        .expect("second");

    let read = store.git_status(&ws()).expect("read");
    assert_eq!(read.changes.len(), 1, "a committed file stayed marked");
    assert_eq!(read.changes[0].path, "/stays.rs");
    assert_eq!(read.changes[0].status, GitStatusKind::Staged);
}

#[test]
fn a_workspace_with_no_git_state_reads_as_no_branch_and_no_changes() {
    // FR-027, and the same answer a workspace that is not a repository gives. Distinguishing
    // them here would put a difference on screen that the developer has no way to act on.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = opened(&dir.path().join("cache.db"));
    let read = store.git_status(&ws()).expect("read");
    assert_eq!(read.branch, BranchPosition::None);
    assert!(read.changes.is_empty());
}

#[test]
fn forgetting_a_workspace_takes_its_git_state_with_it() {
    // The foreign key, asserted rather than assumed. Rows left behind would be handed to the
    // next workspace that happened to reuse the identity.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cache.db");
    let store = opened(&path);
    store
        .replace_git_status(
            &ws(),
            &GitProjection {
                branch: BranchPosition::Branch("main".into()),
                changes: vec![change("/a.rs", GitStatusKind::Modified)],
            },
        )
        .expect("replace");

    store.forget(&ws()).expect("forget");
    store.register(&workspace(), 1).expect("re-register");

    let read = store.git_status(&ws()).expect("read");
    assert!(
        read.changes.is_empty(),
        "git rows outlived the workspace they belonged to"
    );
    assert_eq!(read.branch, BranchPosition::None);
}

#[test]
fn a_detached_head_survives_a_restart_as_a_detached_head() {
    // Not as a branch named after the commit. The three cases are stored distinctly or the
    // status bar will call a commit a branch (FR-018, FR-019).
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cache.db");
    {
        let store = opened(&path);
        store
            .replace_git_status(
                &ws(),
                &GitProjection {
                    branch: BranchPosition::Detached("9f1c2ab".into()),
                    changes: Vec::new(),
                },
            )
            .expect("replace");
    }
    let store = opened(&path);
    assert_eq!(
        store.git_status(&ws()).expect("read").branch,
        BranchPosition::Detached("9f1c2ab".into())
    );
}
