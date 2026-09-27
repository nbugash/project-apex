//! A file changing on the host becomes a frame on the wire.
//!
//! **F004 has no test that makes this claim.** Its suites drive a `FakeWatcher`, which proves
//! the coalescer and the watch bookkeeping and says nothing about whether real inotify events
//! ever reach `workspace/onFileEvent`. F011 found the gap from the other end: the client began
//! consuming these events and received none.

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::watchers::Watchers;
use apex_engine::application::exclusions::ExclusionSet;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::domain::path::ResolvedPath;
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::WorkspaceId;
use common::frames::Sink;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Every `workspace/*` notification that has reached the wire.
fn notifications(sink: &Sink) -> Vec<serde_json::Value> {
    let raw = sink.0.lock().expect("sink").clone();
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
                            found.push(v);
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

fn within(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Watch one path of a real directory, the way `workspace/watch` does.
fn watching(root: &std::path::Path, path: &str) -> (Sink, Watchers, u32) {
    let sink = Sink::default();
    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let canonical = ResolvedPath::canonical_root(root, fs.as_ref()).expect("canonical root");
    let watchers = Watchers::new(
        apex_engine::adapters::outbound::inotify_watcher::factory(),
        Arc::clone(&fs),
        Arc::new(FrameWriter::new(Box::new(sink.clone()))),
        FrameCodec::new(),
        None,
    );
    let result = watchers
        .watch(
            &WorkspaceId("w1".into()),
            &canonical,
            Arc::new(ExclusionSet::resolve(&canonical, fs.as_ref())),
            vec![path.to_string()],
        )
        .expect("the watcher must exist on this host");
    (sink, watchers, result.watching)
}

fn events_naming(sink: &Sink, path: &str) -> bool {
    notifications(sink).iter().any(|n| {
        n["method"] == "workspace/onFileEvent"
            && n["params"]["events"]
                .as_array()
                .is_some_and(|e| e.iter().any(|x| x["relative_path"] == path))
    })
}

#[test]
fn a_file_created_at_the_root_reaches_the_wire() {
    // The exact case F011's tree needs: a new file has no row until an event carries it, and
    // the tree deliberately re-lists nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("existing.rs"), "x\n").unwrap();
    let (sink, _w, watching) = watching(dir.path(), "/");
    assert!(watching >= 1, "the root must be watched");

    std::fs::write(dir.path().join("brand-new.rs"), "fresh\n").unwrap();

    assert!(
        within(Duration::from_secs(5), || events_naming(
            &sink,
            "/brand-new.rs"
        )),
        "no event named the created file; frames were {:?}",
        notifications(&sink)
    );
}

#[test]
fn the_client_s_spelling_of_the_root_is_accepted() {
    // The client asks for `/`, because that is how every other method spells the workspace
    // root. A watcher that accepted only `.` would report `watching: 1` and watch nothing,
    // which is indistinguishable from a host where nothing changed.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.rs"), "x\n").unwrap();
    let (sink, _w, _n) = watching(dir.path(), "/");
    std::fs::write(dir.path().join("a.rs"), "changed\n").unwrap();
    assert!(
        within(Duration::from_secs(5), || events_naming(&sink, "/a.rs")),
        "a modification under a root watched as `/` produced nothing"
    );
}

#[test]
fn paths_on_the_wire_are_rooted_like_every_other_method() {
    // §4.8 writes `/src/controllers/user.go`; `workspace/readDirectory` answers with a leading
    // slash; the client's own path type normalises to one. Events were the single exception,
    // and the mismatch was invisible until something consumed both.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "x\n").unwrap();
    let (sink, _w, _n) = watching(dir.path(), "/src");

    std::fs::write(dir.path().join("src/b.rs"), "y\n").unwrap();

    assert!(within(Duration::from_secs(5), || !notifications(&sink)
        .iter()
        .filter(|n| n["method"] == "workspace/onFileEvent")
        .collect::<Vec<_>>()
        .is_empty()));
    for n in notifications(&sink) {
        if n["method"] != "workspace/onFileEvent" {
            continue;
        }
        for e in n["params"]["events"].as_array().unwrap_or(&Vec::new()) {
            let p = e["relative_path"].as_str().unwrap_or_default();
            assert!(
                p.starts_with('/'),
                "an event carried an unrooted path: {p:?}"
            );
        }
    }
}
