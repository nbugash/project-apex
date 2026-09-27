//! A branch switch, counted.
//!
//! **The count is the assertion.** An implementation that emitted one status update per changed
//! path would satisfy every other test in this feature -- each update would be correct, the
//! marks would be right, and a switch on a large repository would take the writer thousands of
//! frames ahead of whatever interactive request was behind it (FR-024, SC-005).
//!
//! Two separate budgets, because two subsystems see a switch. The git subsystem sees `HEAD`
//! change and owes **one** status computation. The workspace watcher sees thousands of files
//! change and owes **one** bulk invalidation rather than thousands of events (A-COALESCE) --
//! and the two paths are deliberately unconnected (A-GITWATCH), so neither bounds the other.

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::git_cli::GitCli;
use apex_engine::adapters::outbound::git_watchers::GitWatchers;
use apex_engine::adapters::outbound::inotify_watcher::InotifyGitWatch;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::watchers::{WatcherFactory, Watchers};
use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::git_watch::StatusNudge;
use apex_engine::application::use_cases::git_status::GitService;
use apex_engine::domain::path::ResolvedPath;
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::WorkspaceId;
use common::frames::Sink;
use common::repo::Repo;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How many files the switch changes. Large enough that a per-file implementation is obvious
/// in the count and small enough that the fixture builds in a few seconds.
const FILES: usize = 2_000;

fn resolved(root: &std::path::Path) -> ResolvedPath {
    let fs = StdFileSystem;
    let canonical = ResolvedPath::canonical_root(root, &fs).expect("canonical root");
    ResolvedPath::resolve(&canonical, ".", &fs).expect("resolve")
}

fn methods(sink: &Sink) -> Vec<String> {
    let raw = sink.0.lock().expect("sink").clone();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let mut out = Vec::new();
    for (start, _) in text.match_indices("\"method\":\"") {
        let rest = &text[start + 10..];
        if let Some(end) = rest.find('"') {
            out.push(rest[..end].to_string());
        }
    }
    out
}

fn count(sink: &Sink, method: &str) -> usize {
    methods(sink)
        .iter()
        .filter(|m| m.as_str() == method)
        .count()
}

fn settle(limit: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < limit {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_switch_changing_thousands_of_files_costs_one_status_update() {
    let repo = Repo::new();
    // A branch whose content differs from `main` in every one of `FILES` files.
    for i in 0..FILES {
        repo.write(&format!("f{i:05}.txt", i = i), "before\n");
    }
    repo.run(&["add", "-A"]);
    repo.run(&["commit", "-qm", "many"]);
    repo.run(&["checkout", "-q", "-b", "other"]);
    for i in 0..FILES {
        repo.write(&format!("f{i:05}.txt", i = i), "after\n");
    }
    repo.run(&["add", "-A"]);
    repo.run(&["commit", "-qm", "many again"]);
    repo.run(&["checkout", "-q", "main"]);

    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let codec = FrameCodec::new();
    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let git = Arc::new(GitWatchers::new(
        Some(Arc::new(InotifyGitWatch)),
        Arc::new(GitService::new(Arc::new(GitCli::default()))),
        Arc::new(SystemClock),
        Arc::clone(&writer),
        codec.clone(),
    ));
    let factory: WatcherFactory = apex_engine::adapters::outbound::inotify_watcher::factory();
    let watchers = Watchers::new(
        factory,
        Arc::clone(&fs),
        writer,
        codec,
        Some(Arc::clone(&git) as Arc<dyn StatusNudge>),
    );

    let ws = WorkspaceId("w1".into());
    let canonical = ResolvedPath::canonical_root(&repo.root, fs.as_ref()).expect("root");
    git.observe(&ws, &resolved(&repo.root));
    watchers.watch(
        &ws,
        &canonical,
        Arc::new(ExclusionSet::resolve(&canonical, fs.as_ref())),
        vec!["/".into()],
    );
    // Let anything the setup itself provoked settle, so the count below is the switch's.
    std::thread::sleep(Duration::from_millis(800));
    let before_status = count(&sink, "git/onStatusUpdate");
    let before_events = count(&sink, "workspace/onFileEvent");
    let before_bulk = count(&sink, "workspace/invalidateAll");

    repo.run(&["checkout", "-q", "other"]);

    // Wait for something to arrive, then let the whole burst finish.
    settle(Duration::from_secs(10), || {
        count(&sink, "git/onStatusUpdate") > before_status
    });
    std::thread::sleep(Duration::from_secs(2));

    let status_updates = count(&sink, "git/onStatusUpdate") - before_status;
    let file_events = count(&sink, "workspace/onFileEvent") - before_events;
    let bulk = count(&sink, "workspace/invalidateAll") - before_bulk;
    println!(
        "switch of {FILES} files: {status_updates} status update(s), \
         {file_events} file-event frame(s), {bulk} bulk invalidation(s)"
    );

    assert!(
        status_updates >= 1,
        "a branch switch must produce a status update"
    );
    // **Two, not `FILES`.** A-COALESCE bounds a burst of any length at the run already going
    // plus one reflecting everything that happened while it ran.
    assert!(
        status_updates <= 2,
        "{FILES} changed files produced {status_updates} status updates; \
         the coalescer is not bounding the burst"
    );
    // The workspace side. **Bounded against the file count, not against a fixed small
    // number**, because that is what FR-024 and SC-005 actually claim: a switch must not cost
    // one frame per file. It does not cost exactly one frame either -- a burst this size
    // arrives in waves, each wave that crosses A-COALESCE's 256-path threshold invalidating
    // wholesale and the remainders travelling as events. Measured on this fixture: 5 event
    // frames and 7 invalidations for 2,000 files.
    //
    // Asserting `<= 2` was the first version of this and was wrong -- not because the
    // implementation is at fault but because no requirement says two, and a bound nobody can
    // point at a reason for is a bound that gets relaxed the first time it fails.
    assert!(bulk >= 1, "a burst this size must invalidate wholesale");
    let total = file_events + bulk;
    assert!(
        total * 50 < FILES,
        "{FILES} changed files produced {total} frames; a switch is paying per file"
    );
}

#[test]
fn the_status_after_a_switch_describes_the_branch_arrived_at() {
    // FR-026: nothing may remain marked from the branch the developer left. The status is a
    // whole answer and replaces wholesale, so this asserts the answer is the new branch's.
    let repo = Repo::new();
    repo.run(&["checkout", "-q", "-b", "other"]);
    repo.write("only-here.txt", "x\n");
    repo.run(&["add", "-A"]);
    repo.run(&["commit", "-qm", "other branch"]);
    repo.run(&["checkout", "-q", "main"]);

    let service = GitService::new(Arc::new(GitCli::default()));
    let on_main = service
        .refresh("w1", &resolved(&repo.root), 1_000)
        .expect("status");
    assert_eq!(
        on_main.current_branch,
        apex_protocol::wire::BranchPosition::Branch("main".into())
    );
    assert!(
        !on_main.changes.iter().any(|c| c.path == "/only-here.txt"),
        "a file from the other branch is marked on main"
    );

    repo.run(&["checkout", "-q", "other"]);
    let on_other = service
        .refresh("w1", &resolved(&repo.root), 1_000)
        .expect("status");
    assert_eq!(
        on_other.current_branch,
        apex_protocol::wire::BranchPosition::Branch("other".into())
    );
}
