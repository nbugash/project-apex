//! The engine speaks first.
//!
//! FR-002 and FR-003 are requirements about the **absence of a request**: a change made on the
//! host reaches the client without the client having asked. Every other test in this feature
//! inspects a reply, and an implementation that only ever answered would pass all of them while
//! failing the thing the feature is for.
//!
//! So this test never issues a second request. It asks once -- which is what subscribes -- and
//! then changes the repository and waits for the engine to say so.

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::git_cli::GitCli;
use apex_engine::adapters::outbound::git_watchers::GitWatchers;
use apex_engine::adapters::outbound::inotify_watcher::InotifyGitWatch;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::application::use_cases::git_status::GitService;
use apex_engine::domain::path::ResolvedPath;
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::WorkspaceId;
use common::frames::Sink;
use common::repo::Repo;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// SC-001 and SC-002: a change is marked within two seconds. Measured here, where the behaviour
/// is, rather than only tabulated in quickstart.
const BOUND: Duration = Duration::from_secs(2);

fn resolved(root: &std::path::Path) -> ResolvedPath {
    let fs = StdFileSystem;
    let canonical = ResolvedPath::canonical_root(root, &fs).expect("canonical root");
    ResolvedPath::resolve(&canonical, ".", &fs).expect("resolve")
}

fn watchers(sink: &Sink) -> GitWatchers {
    GitWatchers::new(
        Some(Arc::new(InotifyGitWatch)),
        Arc::new(GitService::new(Arc::new(GitCli::default()))),
        Arc::new(SystemClock),
        Arc::new(FrameWriter::new(Box::new(sink.clone()))),
        FrameCodec::new(),
    )
}

/// Every `git/onStatusUpdate` body that has reached the wire so far.
fn updates(sink: &Sink) -> Vec<serde_json::Value> {
    let raw = sink.0.lock().expect("sink").clone();
    // Frame bodies are JSON and every byte of them is ASCII, so scanning the concatenation for
    // object starts finds each body without re-implementing the codec.
    let text = String::from_utf8_lossy(&raw).into_owned();
    let mut found = Vec::new();
    for (start, _) in text.match_indices("{\"jsonrpc\"") {
        let mut depth = 0usize;
        for (offset, ch) in text[start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(
                            &text[start..start + offset + 1],
                        ) {
                            if v.get("method").and_then(|m| m.as_str())
                                == Some("git/onStatusUpdate")
                            {
                                found.push(v);
                            }
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// Wait for a condition, returning how long it took. `None` means it never happened.
fn within(limit: Duration, mut cond: impl FnMut() -> bool) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if cond() {
            return Some(start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn names(update: &serde_json::Value, path: &str) -> bool {
    update["params"]["changes"]
        .as_array()
        .is_some_and(|c| c.iter().any(|e| e["path"] == path))
}

#[test]
fn a_host_change_reaches_the_client_with_no_request() {
    let repo = Repo::new();
    let sink = Sink::default();
    let w = watchers(&sink);
    w.observe(&WorkspaceId("w1".into()), &resolved(&repo.root));

    // The developer edits a file in another terminal. Nothing asks the engine anything.
    repo.write("tracked.rs", "changed\n");

    let took = within(BOUND, || {
        updates(&sink).iter().any(|u| names(u, "/tracked.rs"))
    });
    assert!(
        took.is_some(),
        "no git/onStatusUpdate named the changed file within {BOUND:?}; got {:?}",
        updates(&sink)
    );
}

#[test]
fn staging_reaches_the_client_too() {
    // The case that motivates A-GITWATCH: `git add` writes the index and touches no working-tree
    // file, so a client driven by ordinary file events would never hear about it.
    let repo = Repo::new();
    repo.write("staged.rs", "new\n");
    let sink = Sink::default();
    let w = watchers(&sink);
    w.observe(&WorkspaceId("w1".into()), &resolved(&repo.root));

    repo.run(&["add", "staged.rs"]);

    assert!(
        within(BOUND, || updates(&sink)
            .iter()
            .any(|u| names(u, "/staged.rs")))
        .is_some(),
        "staging must reach the client; got {:?}",
        updates(&sink)
    );
}

#[test]
fn an_update_carries_the_branch() {
    let repo = Repo::new();
    let sink = Sink::default();
    let w = watchers(&sink);
    w.observe(&WorkspaceId("w1".into()), &resolved(&repo.root));
    repo.write("a.rs", "x\n");

    assert!(within(BOUND, || !updates(&sink).is_empty()).is_some());
    let first = updates(&sink).remove(0);
    assert_eq!(first["params"]["current_branch"]["kind"], "branch");
    assert_eq!(first["params"]["workspace_id"], "w1");
}

#[test]
fn a_burst_is_one_update_and_not_one_per_write() {
    // SC-013 observed end to end rather than against a fake clock. The unit test in
    // `git_coalesce.rs` proves the arithmetic; this proves the arithmetic is actually wired to
    // the thing that emits.
    let repo = Repo::new();
    let sink = Sink::default();
    let w = watchers(&sink);
    w.observe(&WorkspaceId("w1".into()), &resolved(&repo.root));

    for i in 0..50 {
        repo.write(&format!("burst{i}.rs"), "x\n");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(within(BOUND, || !updates(&sink).is_empty()).is_some());
    // Let anything still due arrive before counting.
    std::thread::sleep(Duration::from_millis(600));

    let n = updates(&sink).len();
    assert!(
        n <= 4,
        "fifty writes produced {n} updates; the coalescer is not wired to the emitter"
    );
    assert!(
        updates(&sink)
            .last()
            .is_some_and(|u| names(u, "/burst49.rs")),
        "the last update must reflect the end of the burst, not its beginning"
    );
}

#[test]
fn forgetting_a_workspace_stops_the_updates() {
    let repo = Repo::new();
    let sink = Sink::default();
    let w = watchers(&sink);
    let id = WorkspaceId("w1".into());
    w.observe(&id, &resolved(&repo.root));
    repo.write("a.rs", "x\n");
    assert!(within(BOUND, || !updates(&sink).is_empty()).is_some());

    w.forget(&id);
    let after = updates(&sink).len();
    repo.write("b.rs", "y\n");
    std::thread::sleep(Duration::from_millis(500));

    assert_eq!(
        updates(&sink).len(),
        after,
        "a closed workspace must not go on reporting"
    );
}

#[test]
fn observing_twice_does_not_double_the_updates() {
    // The natural caller is every `git/getStatus`, so this is the ordinary path rather than an
    // edge case. Two watches on one repository would double every notification.
    let repo = Repo::new();
    let sink = Sink::default();
    let w = watchers(&sink);
    let id = WorkspaceId("w1".into());
    let root = resolved(&repo.root);
    w.observe(&id, &root);
    w.observe(&id, &root);
    w.observe(&id, &root);

    repo.write("a.rs", "x\n");
    assert!(within(BOUND, || !updates(&sink).is_empty()).is_some());
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        updates(&sink).len() <= 2,
        "observing three times produced {} updates",
        updates(&sink).len()
    );
}

#[test]
fn a_directory_that_is_not_a_repository_is_not_watched_and_does_not_fail() {
    // FR-027. Most directories are not repositories; observing one must be a no-op rather than
    // an error, and must not leave a thread running.
    let (dir, _keep) = common::repo::not_a_repo();
    let sink = Sink::default();
    let w = watchers(&sink);
    w.observe(&WorkspaceId("w1".into()), &resolved(&dir));

    std::fs::write(dir.join("whatever.txt"), "x\n").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        updates(&sink).is_empty(),
        "a directory with no repository reported git state"
    );
}
