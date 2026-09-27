//! Parsing `git status --porcelain=v2 -z`, and the trap in it.
//!
//! Two kinds of case. Against a **real repository**, because the format is git's and a fixture
//! of our own strings would only assert what we already believe. Against **hand-written frames**
//! for the shapes a repository will not produce on demand — a truncated record, an unknown type.
//!
//! The rename case is the one to read. A `2` record is followed by its original path as a
//! separate NUL-terminated field, so a parser that splits on NUL and takes one record per field
//! reads that old path as a new record and desynchronises for everything after it (research.md).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::git_cli::parse_status;
use apex_protocol::wire::{BranchPosition, GitStatusKind};
use common::repo::Repo;

fn state_of(changes: &[apex_protocol::wire::GitChange], path: &str) -> Option<GitStatusKind> {
    changes.iter().find(|c| c.path == path).map(|c| c.status)
}

/// The real thing: every shape at once, out of one stream.
fn every_shape() -> apex_engine::application::ports::git::StatusSnapshot {
    let r = Repo::new();
    r.with_every_shape();
    let raw = r.run(&["status", "--porcelain=v2", "-z", "--branch"]);
    parse_status(&raw).expect("a real repository's status must parse")
}

#[test]
fn a_modified_file_is_modified() {
    assert_eq!(
        state_of(&every_shape().changes, "/src/a.txt"),
        Some(GitStatusKind::Modified)
    );
}

#[test]
fn an_untracked_file_is_untracked() {
    assert_eq!(
        state_of(&every_shape().changes, "/untracked.txt"),
        Some(GitStatusKind::Untracked)
    );
}

#[test]
fn a_staged_file_with_no_further_edit_is_staged() {
    assert_eq!(
        state_of(&every_shape().changes, "/staged.txt"),
        Some(GitStatusKind::Staged)
    );
}

#[test]
fn a_deleted_file_is_deleted() {
    assert_eq!(
        state_of(&every_shape().changes, "/doomed.txt"),
        Some(GitStatusKind::Deleted)
    );
}

#[test]
fn a_conflicted_file_is_conflicted() {
    assert_eq!(
        state_of(&every_shape().changes, "/conflict.txt"),
        Some(GitStatusKind::Conflict)
    );
}

#[test]
fn a_rename_does_not_desynchronise_everything_after_it() {
    // The trap. A `2` record carries its original path as a separate NUL field; a parser that
    // treated every field as a record start would emit a phantom entry for the old path and then
    // misread every record that follows.
    let snap = every_shape();

    assert_eq!(
        state_of(&snap.changes, "/renamed.txt"),
        Some(GitStatusKind::Staged),
        "the rename's new path must be reported: {:?}",
        snap.changes
    );
    assert!(
        state_of(&snap.changes, "/kept.txt").is_none(),
        "the original path of a rename is not a change of its own: {:?}",
        snap.changes
    );
    // And the records that follow it are still intact, which is what desynchronisation destroys.
    assert!(
        state_of(&snap.changes, "/conflict.txt").is_some(),
        "a record after the rename went missing, which is what desynchronisation looks like"
    );
}

#[test]
fn every_path_is_reported_exactly_once() {
    // One state per path (§4.8). A duplicate would mean the collapse happened twice or the
    // parser emitted a phantom, and whichever row the client stored last would win silently.
    let snap = every_shape();
    let mut seen: Vec<&str> = snap.changes.iter().map(|c| c.path.as_str()).collect();
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(before, seen.len(), "duplicate paths: {:?}", snap.changes);
}

#[test]
fn paths_are_workspace_relative_and_rooted() {
    // The client keys on these. A path without a leading separator would key differently from
    // the same file arriving through the tree, and the two would never join.
    for c in every_shape().changes {
        assert!(c.path.starts_with('/'), "not rooted: {}", c.path);
        assert!(!c.path.contains(".."), "not contained: {}", c.path);
    }
}

// ---- The precedence rule, on the two characters rather than on one ----

#[test]
fn the_unstaged_state_wins_over_the_staged_one() {
    // `.M` is unmodified-in-index, modified-in-worktree. `M.` is the reverse. A parser that
    // collapsed the pair before applying the rule would give these the same answer, which is
    // exactly the mistake the rule exists to prevent.
    let unstaged =
        parse_status("# branch.head main\u{0}1 .M N... 100644 100644 100644 aa bb src/a.rs\u{0}")
            .expect("parse");
    let staged =
        parse_status("# branch.head main\u{0}1 M. N... 100644 100644 100644 aa bb src/a.rs\u{0}")
            .expect("parse");

    assert_eq!(
        state_of(&unstaged.changes, "/src/a.rs"),
        Some(GitStatusKind::Modified)
    );
    assert_eq!(
        state_of(&staged.changes, "/src/a.rs"),
        Some(GitStatusKind::Staged)
    );
    assert_ne!(
        state_of(&unstaged.changes, "/src/a.rs"),
        state_of(&staged.changes, "/src/a.rs"),
        "the two characters carry different facts and must not collapse to one answer"
    );
}

#[test]
fn a_file_staged_and_then_edited_again_reports_the_unstaged_state() {
    // `MM`: staged, then edited. The developer's work is recorded nowhere, and reporting STAGED
    // would tell them it is safe when part of it is not (spec.md, *Clarifications*).
    let snap =
        parse_status("# branch.head main\u{0}1 MM N... 100644 100644 100644 aa bb src/a.rs\u{0}")
            .expect("parse");
    assert_eq!(
        state_of(&snap.changes, "/src/a.rs"),
        Some(GitStatusKind::Modified)
    );
}

#[test]
fn a_conflict_overrides_every_other_state() {
    let snap = parse_status(
        "# branch.head main\u{0}u AA N... 100644 100644 100644 100644 aa bb cc both.rs\u{0}",
    )
    .expect("parse");
    assert_eq!(
        state_of(&snap.changes, "/both.rs"),
        Some(GitStatusKind::Conflict)
    );
}

// ---- The branch header ----

#[test]
fn the_branch_comes_from_the_header() {
    let snap = parse_status("# branch.oid abc123\u{0}# branch.head main\u{0}").expect("parse");
    assert_eq!(snap.branch, BranchPosition::Branch("main".into()));
}

#[test]
fn a_detached_head_is_not_a_branch_named_detached() {
    // git writes the literal `(detached)` where a name goes. Read as a name it renders as a
    // peculiarly named branch instead of the absence of one (research.md).
    let snap =
        parse_status("# branch.oid 9581378\u{0}# branch.head (detached)\u{0}").expect("parse");
    assert_eq!(snap.branch, BranchPosition::Detached("9581378".into()));
    assert_ne!(snap.branch, BranchPosition::Branch("(detached)".into()));
}

#[test]
fn a_repository_with_no_commits_still_has_a_branch() {
    // An unborn branch: the name exists, the commit does not. Showing nothing here would hide
    // the branch a developer is about to commit to.
    let snap = parse_status("# branch.oid (initial)\u{0}# branch.head main\u{0}").expect("parse");
    assert_eq!(snap.branch, BranchPosition::Branch("main".into()));
}

// ---- Refusing what it cannot read ----

#[test]
fn an_unreadable_record_rejects_the_whole_snapshot() {
    // Not a partial parse. A half-read status is indistinguishable from a repository where the
    // missing files are clean, so the client would be told they are — silently and wrongly.
    assert!(
        parse_status("# branch.head main\u{0}1 .M N... only three fields\u{0}").is_err(),
        "a malformed record must reject the snapshot rather than be skipped"
    );
}

#[test]
fn an_unknown_record_type_is_refused_rather_than_ignored() {
    // A newer git emitting a record this build does not know is a reason to say so, not to
    // report a status that quietly omits whatever it described.
    assert!(parse_status("# branch.head main\u{0}z something\u{0}").is_err());
}

#[test]
fn an_empty_status_is_a_clean_repository_and_not_an_error() {
    let snap = parse_status("# branch.head main\u{0}").expect("parse");
    assert!(snap.changes.is_empty());
    assert_eq!(snap.branch, BranchPosition::Branch("main".into()));
}

// ---- Containment (contracts/git-status.md guarantee 7, Principle VI) ----

#[test]
fn a_path_that_escapes_the_workspace_is_dropped_rather_than_forwarded() {
    // The guarantee says *dropped*. Anything else asks the client to be the only check, which
    // is the one-sided boundary Principle VI exists to prevent.
    let snap = parse_status(
        "# branch.head main\u{0}1 .M N... 100644 100644 100644 aa bb ../outside.rs\u{0}\
         1 .M N... 100644 100644 100644 aa bb inside.rs\u{0}",
    )
    .expect("parse");
    let paths: Vec<&str> = snap.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, vec!["/inside.rs"], "an escaping path was forwarded");
}

#[test]
fn an_absolute_path_is_dropped_rather_than_read_as_workspace_relative() {
    // The hole the previous normaliser left: it split on `/` and reassembled, so `/etc/passwd`
    // and `etc/passwd` both came out as `/etc/passwd` -- the first silently re-read as though
    // it named a file inside the workspace. git reports repository-relative paths, so an
    // absolute one means something upstream is wrong, and repairing it is not this layer's job.
    let snap = parse_status(
        "# branch.head main\u{0}1 .M N... 100644 100644 100644 aa bb /etc/passwd\u{0}",
    )
    .expect("parse");
    assert!(
        snap.changes.is_empty(),
        "an absolute path was reinterpreted as workspace-relative: {:?}",
        snap.changes
    );
}

// ---- A workspace inside a repository (FR-003a) ----

#[test]
fn a_prefix_is_stripped_so_paths_are_workspace_relative() {
    // git reports relative to the **repository** root. A workspace on `services/checkout`
    // needs `/pay.rs`, not `/services/checkout/pay.rs`: the second names nothing that exists
    // in this workspace, so every mark would land on no row at all.
    let snap = apex_engine::adapters::outbound::git_cli::parse_status_in(
        "# branch.head main\u{0}\
         1 .M N... 100644 100644 100644 aa bb services/checkout/pay.rs\u{0}\
         ? services/checkout/scratch.txt\u{0}",
        "services/checkout/",
    )
    .expect("parse");
    let paths: Vec<&str> = snap.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, vec!["/pay.rs", "/scratch.txt"]);
}

#[test]
fn a_path_outside_the_workspace_is_dropped_rather_than_re_rooted() {
    // With `-- .` scoping this should not arrive; if it does it names another team's file, and
    // contracts/git-status.md guarantee 7 says the entry is dropped. Stripping "whatever
    // prefix it happens to have" would put somebody else's path in this tree.
    let snap = apex_engine::adapters::outbound::git_cli::parse_status_in(
        "# branch.head main\u{0}\
         1 .M N... 100644 100644 100644 aa bb services/billing/bill.rs\u{0}\
         1 .M N... 100644 100644 100644 aa bb services/checkout/pay.rs\u{0}",
        "services/checkout/",
    )
    .expect("parse");
    let paths: Vec<&str> = snap.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["/pay.rs"],
        "a file outside the workspace leaked in"
    );
}

#[test]
fn a_rename_inside_a_subdirectory_workspace_is_re_rooted_too() {
    // Record type `2` carries the destination in the same position as a `1`, so the prefix has
    // to be stripped there as well -- and the original path is a separate NUL field that must
    // still be consumed, or everything after it desynchronises.
    let snap = apex_engine::adapters::outbound::git_cli::parse_status_in(
        "# branch.head main\u{0}\
         2 R. N... 100644 100644 100644 aa bb R100 services/checkout/new.rs\u{0}services/checkout/old.rs\u{0}\
         1 .M N... 100644 100644 100644 aa bb services/checkout/after.rs\u{0}",
        "services/checkout/",
    )
    .expect("parse");
    let paths: Vec<&str> = snap.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, vec!["/new.rs", "/after.rs"]);
}

#[test]
fn an_empty_prefix_leaves_paths_exactly_as_they_are() {
    // The ordinary case, and the one every other test in this file exercises.
    let with = apex_engine::adapters::outbound::git_cli::parse_status_in(
        "# branch.head main\u{0}1 .M N... 100644 100644 100644 aa bb src/a.rs\u{0}",
        "",
    )
    .expect("parse");
    let without =
        parse_status("# branch.head main\u{0}1 .M N... 100644 100644 100644 aa bb src/a.rs\u{0}")
            .expect("parse");
    assert_eq!(with.changes, without.changes);
    assert_eq!(with.changes[0].path, "/src/a.rs");
}
