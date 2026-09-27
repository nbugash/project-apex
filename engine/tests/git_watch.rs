//! The two watches, against a real repository.
//!
//! Real inotify and a real git, because what is under test is whether the kernel tells us about
//! the two files git actually writes — a question no fake can answer. The *decisions* built on
//! these events (when to run git, how often) are tested against a fake clock in
//! `git_coalesce.rs`, with no filesystem at all.

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::git_cli::GitCli;
use apex_engine::adapters::outbound::inotify_watcher::InotifyGitWatch;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::git::Git;
use apex_engine::application::ports::git_watch::GitWatch;
use apex_engine::domain::path::ResolvedPath;
use common::repo::Repo;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn resolved(root: &std::path::Path) -> ResolvedPath {
    let fs = StdFileSystem;
    let canonical = ResolvedPath::canonical_root(root, &fs).expect("canonical root");
    ResolvedPath::resolve(&canonical, ".", &fs).expect("resolve")
}

/// Wait for a condition, or give up. Returns whether it happened.
fn within(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// A watch on `repo`'s git directory, and a counter of how often it fired.
fn watching(
    root: &std::path::Path,
) -> (
    Arc<AtomicUsize>,
    Box<dyn apex_engine::application::ports::git_watch::GitWatchHandle>,
) {
    let git_dir = GitCli::default()
        .git_dir(&resolved(root))
        .expect("git_dir must resolve");
    let fired = Arc::new(AtomicUsize::new(0));
    let counter = fired.clone();
    let handle = InotifyGitWatch
        .watch(
            &git_dir,
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .expect("watch");
    (fired, handle)
}

#[test]
fn staging_a_file_fires_the_watch() {
    // The case the workspace watcher cannot see: `git add` writes the index and touches no
    // working-tree file, so an implementation driven by ordinary file events learns nothing and
    // the tree's staged colouring stays wrong until something unrelated changes (FR-003).
    let repo = Repo::new();
    let (fired, _h) = watching(&repo.root);

    repo.write("staged.txt", "new\n");
    repo.run(&["add", "staged.txt"]);

    assert!(
        within(Duration::from_secs(5), || fired.load(Ordering::SeqCst) > 0),
        "an index write must wake the watch"
    );
}

#[test]
fn switching_branches_fires_the_watch() {
    // The other watched file. HEAD changes on a switch, and without it the client would go on
    // showing the branch the developer left (FR-002).
    let repo = Repo::new();
    let (fired, _h) = watching(&repo.root);

    repo.run(&["checkout", "-q", "-b", "elsewhere"]);

    assert!(
        within(Duration::from_secs(5), || fired.load(Ordering::SeqCst) > 0),
        "a branch switch must wake the watch"
    );
}

#[test]
fn a_linked_worktree_is_watched_where_its_git_directory_actually_is() {
    // FR-004, and the test an implementation assuming `<root>/.git` fails while passing
    // everything else. In a linked worktree that path is a *file*; the directory holding this
    // worktree's own HEAD and index is under the main repository's `.git/worktrees/`.
    let repo = Repo::new();
    let wt = repo.add_worktree("wt");

    let git_dir = GitCli::default()
        .git_dir(&resolved(&wt))
        .expect("the worktree's git dir must resolve");
    assert!(
        git_dir.join("index").exists(),
        "the resolved git dir must be the one holding this worktree's index: {git_dir:?}"
    );
    assert_ne!(
        git_dir,
        wt.join(".git"),
        "a worktree's .git is a file; resolving must have gone elsewhere"
    );

    let (fired, _h) = watching(&wt);
    std::fs::write(wt.join("in-worktree.txt"), "x\n").unwrap();
    common::repo::git(&wt, &["add", "in-worktree.txt"]);

    assert!(
        within(Duration::from_secs(5), || fired.load(Ordering::SeqCst) > 0),
        "staging inside a linked worktree must wake its watch"
    );
}

#[test]
fn the_git_watch_produces_no_workspace_file_events() {
    // A-GITWATCH's separation, asserted rather than assumed. The guarantee is that there is no
    // code path from these events to `workspace/onFileEvent` — so the check is structural: the
    // git watch is reached through a port that has no way to express a file event, and its
    // adapter never constructs one.
    //
    // Asserted on the source, because the property is the *absence* of a path. A runtime test
    // could only show that none happened to be emitted during one run.
    let adapter = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/adapters/outbound/inotify_watcher.rs"
    ))
    .expect("the adapter source");
    let git_half = adapter
        .split("---- The git directory watch")
        .nth(1)
        .expect("the git watch section exists");

    // Comments are not uses. The first version of this check failed on its own doc comment --
    // the sentence explaining that this code emits no `RawEvent` -- which is precisely what
    // `inotify_confinement.rs` warns about: a guard that fires on its own rationale teaches
    // people to delete the rationale.
    let code: String = git_half
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in ["RawEvent", "FileEvent", "ExclusionSet", "WatchId"] {
        assert!(
            !code.contains(forbidden),
            "the git watch names {forbidden}; A-GITWATCH's separation is only worth its extra \
             component while these events cannot become workspace events"
        );
    }
}

#[test]
fn dropping_the_handle_stops_the_watch() {
    // A watch outliving its workspace would keep a thread and a descriptor per repository ever
    // opened, and would go on waking a coalescer nobody is reading.
    let repo = Repo::new();
    let (fired, handle) = watching(&repo.root);
    repo.write("a.txt", "1\n");
    repo.run(&["add", "a.txt"]);
    assert!(within(Duration::from_secs(5), || fired
        .load(Ordering::SeqCst)
        > 0));

    drop(handle);
    let after_stop = fired.load(Ordering::SeqCst);
    repo.write("b.txt", "2\n");
    repo.run(&["add", "b.txt"]);
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(
        fired.load(Ordering::SeqCst),
        after_stop,
        "the watch fired after its handle was dropped"
    );
}
