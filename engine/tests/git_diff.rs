//! Which lines differ, as coordinates and never as content.
//!
//! Two independent claims. The **parsing** claim is about `--unified=0` hunk headers, where a
//! count of 1 is elided: `@@ -2 +2 @@` carries two numbers where a reader expecting four finds
//! two, and silently mis-reads every single-line change in the file (research.md, FR-020).
//!
//! The **content** claim is §12.3's, and it is asserted on the payload rather than on the
//! parser. A parser that discards text and a result that carries it look identical from the
//! parser's side, so the only way to make the claim is to look at what a client would receive.

mod common;

use apex_engine::adapters::outbound::git_cli::{parse_diff, GitCli};
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::git::Git;
use apex_engine::domain::path::ResolvedPath;
use common::repo::Repo;

fn resolved(root: &std::path::Path) -> ResolvedPath {
    let fs = StdFileSystem;
    let canonical = ResolvedPath::canonical_root(root, &fs).expect("canonical root");
    ResolvedPath::resolve(&canonical, ".", &fs).expect("resolve")
}

// ---- Hunk headers ----

#[test]
fn an_elided_count_means_one_line() {
    // `@@ -2 +2 @@`: both counts are 1 and both are omitted. A parser assuming `start,count`
    // on each side reads no count, and every single-line edit -- the most common edit there is
    // -- is mishandled.
    let d = parse_diff("@@ -2 +2 @@\n");
    assert_eq!(d.modified, vec![[2, 2]]);
    assert!(d.added.is_empty() && d.deleted.is_empty());
}

#[test]
fn a_zero_count_on_the_old_side_is_an_insertion() {
    // `@@ -4,0 +5,3 @@`: nothing was there, three lines are now.
    let d = parse_diff("@@ -4,0 +5,3 @@\n");
    assert_eq!(d.added, vec![[5, 7]]);
    assert!(d.modified.is_empty());
}

#[test]
fn a_zero_count_on_the_new_side_is_a_deletion_at_a_position() {
    // A deletion is a **position**, not a range: the removed lines are not in the new file, so
    // there is nothing to draw a range over. A zero-length range would render as nothing at
    // all, which is the same as not reporting the deletion.
    let d = parse_diff("@@ -7,3 +6,0 @@\n");
    assert_eq!(d.deleted, vec![6]);
    assert!(d.added.is_empty() && d.modified.is_empty());
}

#[test]
fn several_hunks_are_all_reported() {
    let d = parse_diff("@@ -1 +1 @@\nsome text\n@@ -10,0 +11,2 @@\nmore text\n@@ -20,2 +21,0 @@\n");
    assert_eq!(d.modified, vec![[1, 1]]);
    assert_eq!(d.added, vec![[11, 12]]);
    assert_eq!(d.deleted, vec![21]);
}

#[test]
fn a_line_that_is_not_a_hunk_header_is_ignored() {
    // Everything between headers is the content `--unified=0` still prints for changed lines,
    // and it is discarded here rather than transmitted.
    let d = parse_diff(
        "diff --git a/x b/x\nindex aaa..bbb 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-secret\n+other\n",
    );
    assert_eq!(d.modified, vec![[1, 1]]);
}

#[test]
fn a_malformed_header_is_skipped_rather_than_guessed_at() {
    let d = parse_diff("@@ nonsense @@\n@@ -1 +1 @@\n");
    assert_eq!(d.modified, vec![[1, 1]], "the good hunk still reads");
}

// ---- The content guarantee ----

#[test]
fn the_result_carries_no_file_content_in_any_field() {
    // FR-021, SC-010, §12.3. Asserted on the **payload**: the secret is on both sides of the
    // change, so any field that leaked content would contain it.
    let repo = Repo::new();
    repo.write("secret.txt", "PASSWORD=hunter2\nkeep\n");
    repo.run(&["add", "-A"]);
    repo.run(&["commit", "-qm", "add secret"]);
    repo.write("secret.txt", "PASSWORD=swordfish\nkeep\n");

    let diff = GitCli::default()
        .file_diff(&resolved(&repo.root), "secret.txt")
        .expect("diff");
    let payload = serde_json::to_string(&diff).expect("serialise");

    assert!(
        !payload.contains("hunter2") && !payload.contains("swordfish"),
        "the diff payload carried file content: {payload}"
    );
    assert!(
        !payload.contains("PASSWORD"),
        "the diff payload carried file content: {payload}"
    );
    // And it did report the change, so the assertion above is not passing vacuously.
    assert_eq!(diff.modified, vec![[1, 1]]);
}

#[test]
fn an_unmodified_file_returns_three_empty_lists_and_not_an_error() {
    // FR-022. An error here would make opening an unchanged file -- the ordinary case -- look
    // like a failure.
    let repo = Repo::new();
    let diff = GitCli::default()
        .file_diff(&resolved(&repo.root), "kept.txt")
        .expect("an unmodified file is not an error");
    assert!(diff.added.is_empty() && diff.modified.is_empty() && diff.deleted.is_empty());
}

#[test]
fn an_untracked_file_has_every_line_added() {
    // Nothing has been recorded for it to differ from, so all of it is new.
    let repo = Repo::new();
    repo.write("fresh.txt", "one\ntwo\nthree\n");
    let diff = GitCli::default()
        .file_diff(&resolved(&repo.root), "fresh.txt")
        .expect("diff");
    assert_eq!(
        diff.added,
        vec![[1, 3]],
        "every line of a new file is added"
    );
    assert!(diff.modified.is_empty() && diff.deleted.is_empty());
}

#[test]
fn a_deleted_file_reports_its_lines_as_removed() {
    let repo = Repo::new();
    repo.remove("doomed.txt");
    let diff = GitCli::default()
        .file_diff(&resolved(&repo.root), "doomed.txt")
        .expect("diff");
    assert!(
        !diff.deleted.is_empty(),
        "a removed file's lines must be reported as deleted"
    );
}
