//! US1.4 and US1.5: where a task runs, and what it runs with.
//!
//! Driven through `start_task` rather than through the runner, because the two things under test
//! are the use case's: resolving `cwd` against the workspace root, and merging the caller's `env`
//! **over** the inherited environment rather than replacing it.
//!
//! The `PATH` assertion is the one that discriminates. An implementation that replaces the
//! environment instead of merging into it passes every other case here, because the fixtures are
//! spawned by absolute path and never need `PATH` to run at all. Without that half, US1.5 is
//! satisfied by a build that loses every inherited variable.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::ports::task_runner::ReadOutcome;
use apex_engine::application::use_cases::task::start_task;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::TaskSet;
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

/// Run `fixture_report_env` under `cwd` and `env`, and return everything it printed.
fn report(
    cwd: Option<&str>,
    env: Option<BTreeMap<String, String>>,
    root: &std::path::Path,
) -> String {
    let fs: Arc<dyn apex_engine::application::ports::file_system::FileSystem> =
        Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots
        .register("ws1", root.to_str().expect("utf8 root"))
        .expect("register");

    let runner = PtyRunner::new();
    let mut tasks = TaskSet::default();
    let params = RunTaskParams {
        workspace_id: WorkspaceId("ws1".into()),
        task_id: TaskId("env".into()),
        command: vec![fixture("fixture_report_env")],
        cwd: cwd.map(str::to_string),
        env,
        // Pipes: this is about the environment, and a terminal would rewrite the line endings
        // the assertions read.
        pty: false,
        cols: None,
        rows: None,
    };
    let (_pid, mut spawned) =
        start_task(&params, &roots, fs.as_ref(), &runner, &mut tasks).expect("start");

    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match spawned.output.read(20, &mut out) {
            ReadOutcome::Ended => break,
            ReadOutcome::Failed(e) => panic!("read failed: {e:?}"),
            _ => {}
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A workspace root with one subdirectory in it, so "here" and "the root" are distinguishable.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    dir
}

/// `/tmp` is a symlink on some systems, so the fixture's own view of its directory is
/// canonicalised before comparison. Comparing against the uncanonicalised path would fail for a
/// reason that has nothing to do with the behaviour under test.
fn canonical(p: &std::path::Path) -> String {
    std::fs::canonicalize(p)
        .expect("canonicalize")
        .display()
        .to_string()
}

#[test]
fn a_task_with_a_cwd_runs_there_and_not_in_the_root() {
    // US1.4. The distinguishing case: a `cwd` that is not the root, so an implementation
    // ignoring the field cannot pass by accident.
    let ws = workspace();
    let text = report(Some("sub"), None, ws.path());
    let expected = canonical(&ws.path().join("sub"));
    assert!(
        text.contains(&format!("CWD {expected}")),
        "expected the task to run in {expected}: {text}"
    );
    assert!(
        !text.contains(&format!("CWD {}\n", canonical(ws.path()))),
        "the task ran in the root, ignoring its cwd: {text}"
    );
}

#[test]
fn a_task_with_no_cwd_runs_in_the_workspace_root() {
    // US1.4's other half. `cwd` is optional and the root is the documented default.
    let ws = workspace();
    let text = report(None, None, ws.path());
    let expected = canonical(ws.path());
    assert!(
        text.contains(&format!("CWD {expected}")),
        "expected the task to run in {expected}: {text}"
    );
}

#[test]
fn a_supplied_variable_reaches_the_process() {
    // US1.5's easy half.
    let ws = workspace();
    let mut env = BTreeMap::new();
    env.insert("APEX_FIXTURE_KEY".to_string(), "carried".to_string());
    let text = report(None, Some(env), ws.path());
    assert!(
        text.contains("APEX_FIXTURE_KEY carried"),
        "the supplied variable did not arrive: {text}"
    );
}

#[test]
fn path_survives_a_supplied_environment() {
    // US1.5's half that discriminates. §4.8 merges the caller's `env` **over** the inherited
    // environment; an implementation that replaces it loses PATH and still passes every other
    // case in this file, because the fixture is spawned by absolute path.
    let ws = workspace();
    let mut env = BTreeMap::new();
    env.insert("APEX_FIXTURE_KEY".to_string(), "carried".to_string());
    let text = report(None, Some(env), ws.path());
    assert!(
        text.contains("PATH set"),
        "PATH did not survive, so the environment was replaced rather than merged: {text}"
    );
}
