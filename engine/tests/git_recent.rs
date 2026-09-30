//! `git/recentlyChanged`: which files the last few commits touched, for prefetch (US5).
//!
//! `contracts/recently-changed.md` states six guarantees and three refusals, and this file tests
//! each against a real git and a real repository, because every one of them is a claim about what
//! git's output becomes on the wire. The parser alone is tested once, for deduplication; after that
//! the assertions are made on what `dispatch` actually replies, since a parser that discarded extra
//! fields and a reply that carried them look identical from the parser's side (T063a).

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::git_cli::{parse_recent, GitCli};
use apex_engine::adapters::outbound::git_watchers::GitWatchers;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::git_status::GitService;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::{FrameCodec, MAX_FRAME_BYTES};
use common::frames::Sink;
use common::repo::{git, not_a_repo, Repo};
use std::path::Path;
use std::sync::Arc;

struct Engine {
    git: GitWatchers,
    roots: InMemoryRoots,
    fs: Arc<dyn FileSystem>,
    codec: FrameCodec,
}

fn engine_with(program: GitCli) -> Engine {
    let codec = FrameCodec::new();
    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    Engine {
        git: GitWatchers::new(
            None,
            Arc::new(GitService::new(Arc::new(program))),
            Arc::new(SystemClock),
            Arc::new(FrameWriter::new(Box::new(Sink::default()))),
            codec.clone(),
        ),
        roots: InMemoryRoots::new(Arc::clone(&fs)),
        fs,
        codec,
    }
}

fn engine() -> Engine {
    engine_with(GitCli::default())
}

impl Engine {
    fn register(&self, id: &str, root: &Path) {
        self.roots
            .register(id, root.to_str().expect("utf-8 path"))
            .expect("register");
    }

    /// The whole reply, and the frame's size on the wire.
    fn ask(&self, params: serde_json::Value) -> (serde_json::Value, usize) {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "1", "method": "git/recentlyChanged", "params": params
        })
        .to_string();
        let Action::Reply(frame) = dispatch(
            &SessionRegistry::new(),
            &self.roots,
            self.fs.as_ref(),
            None,
            None,
            Some(&self.git),
            &self.codec,
            &body,
        ) else {
            panic!("a request must be answered");
        };
        let text = String::from_utf8_lossy(&frame).into_owned();
        let json = text.split_once("\r\n\r\n").expect("framed").1;
        (serde_json::from_str(json).expect("json"), frame.len())
    }

    fn paths(&self, params: serde_json::Value) -> Vec<String> {
        let (reply, _) = self.ask(params);
        assert!(reply.get("error").is_none(), "refused: {reply}");
        let mut paths: Vec<String> = reply["result"]["paths"]
            .as_array()
            .expect("paths")
            .iter()
            .map(|p| p.as_str().expect("a path is a string").to_string())
            .collect();
        paths.sort();
        paths
    }

    fn error_code(&self, params: serde_json::Value) -> i64 {
        let (reply, _) = self.ask(params);
        reply["error"]["code"]
            .as_i64()
            .unwrap_or_else(|| panic!("not refused: {reply}"))
    }
}

fn commit(repo: &Repo, rel: &str, body: &str) {
    repo.write(rel, body);
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", rel]);
}

/// Guarantee 4: deduplicated. The parser meets a path once per commit that touched it.
#[test]
fn a_file_changed_in_five_commits_appears_once() {
    // `--pretty=format:` output: one path per line, a blank line between commits.
    let raw = "src/x.rs\n\nsrc/x.rs\nsrc/y.rs\n\nsrc/x.rs\n\nsrc/x.rs\n\nsrc/x.rs\n";
    assert_eq!(parse_recent(raw, ""), vec!["/src/x.rs", "/src/y.rs"]);

    let repo = Repo::new();
    for i in 0..5 {
        commit(&repo, "hot.txt", &format!("{i}\n"));
    }
    let e = engine();
    e.register("w", &repo.root);
    let paths = e.paths(serde_json::json!({ "workspace_id": "w" }));
    assert_eq!(
        paths.iter().filter(|p| *p == "/hot.txt").count(),
        1,
        "{paths:?}"
    );
}

/// Guarantee 3 and F011's FR-003a: a workspace on a subdirectory receives its own paths re-rooted,
/// and nothing from elsewhere in the repository.
#[test]
fn a_subdirectory_workspace_receives_only_its_own_paths_re_rooted() {
    let repo = Repo::new();
    commit(&repo, "svc/inside.rs", "mine\n");
    commit(&repo, "other/outside.rs", "theirs\n");
    let e = engine();
    e.register("w", &repo.root.join("svc"));

    let paths = e.paths(serde_json::json!({ "workspace_id": "w" }));

    assert_eq!(paths, vec!["/inside.rs"]);
}

/// Guarantee 5: not a repository, no git on the host, and a repository with no commits yet all
/// answer successfully with an empty list.
#[test]
fn no_repository_no_git_and_no_commits_are_each_an_empty_success() {
    let (plain, _keep) = not_a_repo();
    let e = engine();
    e.register("plain", &plain);
    assert!(e
        .paths(serde_json::json!({ "workspace_id": "plain" }))
        .is_empty());

    let repo = Repo::new();
    let no_git = engine_with(GitCli::new("apex-no-such-git-binary"));
    no_git.register("w", &repo.root);
    assert!(no_git
        .paths(serde_json::json!({ "workspace_id": "w" }))
        .is_empty());

    let empty = Repo::empty();
    e.register("fresh", &empty.root);
    assert!(e
        .paths(serde_json::json!({ "workspace_id": "fresh" }))
        .is_empty());
}

/// Guarantee 2: `commits` defaults to 20 and is capped at 100. And the refusal for a count that is
/// not positive.
#[test]
fn commits_default_to_twenty_and_are_capped_at_one_hundred() {
    let repo = Repo::new();
    // One distinct file per commit, so the number of paths is the number of commits walked.
    for i in 0..105 {
        commit(&repo, &format!("c/{i:03}.txt"), "x\n");
    }
    let e = engine();
    e.register("w", &repo.root);

    assert_eq!(
        e.paths(serde_json::json!({ "workspace_id": "w" })).len(),
        20
    );
    assert_eq!(
        e.paths(serde_json::json!({ "workspace_id": "w", "commits": 5 }))
            .len(),
        5
    );
    assert_eq!(
        e.paths(serde_json::json!({ "workspace_id": "w", "commits": 500 }))
            .len(),
        100,
        "a caller asking for more gets 100"
    );
    assert_eq!(
        e.error_code(serde_json::json!({ "workspace_id": "w", "commits": 0 })),
        -32602
    );
    assert_eq!(
        e.error_code(serde_json::json!({ "workspace_id": "w", "commits": -3 })),
        -32602
    );
}

/// Guarantee 1, asserted on the serialised payload: paths only -- no content, no commit identity,
/// no author, no date.
#[test]
fn the_reply_carries_paths_and_nothing_else() {
    let repo = Repo::new();
    commit(&repo, "secret.txt", "a credential, perhaps\n");
    let head = git(&repo.root, &["rev-parse", "HEAD"]).trim().to_string();
    let e = engine();
    e.register("w", &repo.root);

    let (reply, _) = e.ask(serde_json::json!({ "workspace_id": "w" }));
    let result = reply["result"].as_object().expect("an object");

    assert_eq!(result.keys().collect::<Vec<_>>(), vec!["paths"]);
    let text = reply.to_string();
    for leak in [
        head.as_str(),
        &head[..7],
        "credential",
        "Fixture",
        "example.invalid",
    ] {
        assert!(!text.contains(leak), "the reply carries {leak:?}: {text}");
    }
    assert!(result["paths"]
        .as_array()
        .expect("paths")
        .iter()
        .all(|p| p.as_str().is_some_and(|s| s.starts_with('/'))));
}

/// Guarantee 6: a result that would exceed the frame cap is truncated rather than refused, and no
/// cursor is offered. Driven by a commit that genuinely touches more path bytes than one frame
/// holds, so the truncation branch really executes.
#[test]
fn a_result_larger_than_a_frame_is_truncated_not_refused_and_not_paged() {
    let repo = Repo::new();
    // 5,000 paths of about 230 bytes each: well over 1 MiB of names.
    let dir = "d".repeat(100);
    std::fs::create_dir_all(repo.root.join(&dir)).expect("dir");
    for i in 0..5_000 {
        let name = format!("{dir}/{i:05}-{}", "n".repeat(120));
        std::fs::write(repo.root.join(name), "x\n").expect("write");
    }
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-qm", "many"]);
    let e = engine();
    e.register("w", &repo.root);

    let (reply, frame) = e.ask(serde_json::json!({ "workspace_id": "w", "commits": 1 }));

    assert!(
        reply.get("error").is_none(),
        "refused rather than truncated: {reply}"
    );
    assert!(
        frame <= MAX_FRAME_BYTES,
        "the reply exceeds the frame cap: {frame}"
    );
    let n = reply["result"]["paths"].as_array().expect("paths").len();
    assert!(
        n > 0 && n < 5_000,
        "truncated to something, not to nothing: {n}"
    );
    assert_eq!(
        reply["result"].as_object().expect("obj").len(),
        1,
        "no cursor offered"
    );
}

/// The contract's refusals: `-32001` for a workspace never registered, and `-32009` -- not `-32001`,
/// and not a successful empty list -- for one whose directory has gone. The empty list is the trap:
/// guarantee 5 makes it the right answer for a directory that is not a repository, so an
/// implementation that answers empty for a *missing* directory passes every other test here.
#[test]
fn an_unregistered_workspace_and_a_gone_root_are_refused_differently() {
    let e = engine();
    assert_eq!(
        e.error_code(serde_json::json!({ "workspace_id": "never" })),
        -32001
    );

    let doomed = tempfile::tempdir().expect("temp");
    let root = doomed.path().join("ws");
    std::fs::create_dir_all(&root).expect("dir");
    e.register("w", &root);
    std::fs::remove_dir_all(&root).expect("remove");

    assert_eq!(
        e.error_code(serde_json::json!({ "workspace_id": "w" })),
        -32009
    );
}
