//! A task's working directory is contained, and a live identity is refused before anything runs
//! (T046, FR-003, FR-031c, §4.7).

mod common;

use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::task::{
    start_task, StartRefusal, DEFAULT_COLS, DEFAULT_ROWS,
};
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::{Shape, TaskSet};
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::fake_runner::{FakeRunner, Script};
use common::FakeFileSystem;
use std::collections::BTreeMap;
use std::sync::Arc;

fn setup() -> (Arc<FakeFileSystem>, InMemoryRoots, FakeRunner, TaskSet) {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    fs.dir("/w/crates");
    fs.dir("/outside");
    let roots = InMemoryRoots::new(fs.clone());
    roots.register("ws-1", "/w").expect("register");
    (fs, roots, FakeRunner::new(), TaskSet::new())
}

fn params(cwd: Option<&str>) -> RunTaskParams {
    RunTaskParams {
        workspace_id: WorkspaceId("ws-1".into()),
        task_id: TaskId("build".into()),
        command: vec!["cargo".into(), "test".into()],
        cwd: cwd.map(str::to_string),
        env: None,
        pty: true,
        cols: None,
        rows: None,
    }
}

#[test]
fn a_cwd_inside_the_root_is_accepted() {
    let (fs, roots, runner, mut tasks) = setup();
    runner.script(Script::of(vec![]));
    let result = start_task(
        &params(Some("crates")),
        &roots,
        fs.as_ref(),
        &runner,
        &mut tasks,
    );
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(runner.spawns().len(), 1);
}

#[test]
fn every_escape_shape_is_refused_identically() {
    // FR-007's reasoning applied to `cwd`: a caller able to tell "outside and exists" from
    // "outside and does not" could probe the host's filesystem using nothing but refusals.
    for escape in ["..", "../outside", "crates/../../outside", "..\\outside"] {
        let (fs, roots, runner, mut tasks) = setup();
        let result = start_task(
            &params(Some(escape)),
            &roots,
            fs.as_ref(),
            &runner,
            &mut tasks,
        );
        assert_eq!(
            result.err(),
            Some(StartRefusal::PathRefused),
            "escape shape {escape:?} was not refused identically"
        );
        // And nothing was spawned. A refusal after the fork would be a process the developer
        // never asked for, running in a directory they are not allowed to reach.
        assert!(runner.spawns().is_empty(), "{escape:?} spawned something");
    }
}

/// A leading slash is not an escape, and the behaviour is worth pinning because it surprises.
#[test]
fn an_absolute_looking_path_resolves_inside_the_root() {
    let (fs, roots, runner, mut tasks) = setup();
    fs.dir("/w/outside");
    runner.script(Script::of(vec![]));

    // §4.7: every path in every method is **relative to the workspace root**. So `/outside`
    // names `<root>/outside`, not the host's `/outside`, and the leading slash is an empty
    // segment rather than an anchor. A reader expecting a refusal here would be reading it as a
    // host path, which is exactly the reading §4.7 removes -- and the safe outcome either way,
    // since there is no spelling of an absolute path that reaches outside the root.
    let result = start_task(
        &params(Some("/outside")),
        &roots,
        fs.as_ref(),
        &runner,
        &mut tasks,
    );
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(runner.spawns().len(), 1);
}

#[test]
fn a_missing_directory_inside_the_root_is_a_miss_rather_than_a_refusal() {
    let (fs, roots, runner, mut tasks) = setup();
    let result = start_task(
        &params(Some("no-such-dir")),
        &roots,
        fs.as_ref(),
        &runner,
        &mut tasks,
    );
    // Inside the root and absent: information the caller is entitled to (-32003), and
    // deliberately distinguishable from an escape.
    assert_eq!(result.err(), Some(StartRefusal::NotFound));
}

#[test]
fn a_live_identity_is_refused_before_anything_is_spawned() {
    let (fs, roots, runner, mut tasks) = setup();
    runner.script(Script::of(vec![]));
    start_task(&params(None), &roots, fs.as_ref(), &runner, &mut tasks).expect("first start");
    assert_eq!(runner.spawns().len(), 1);

    let again = start_task(&params(None), &roots, fs.as_ref(), &runner, &mut tasks);
    assert_eq!(again.err(), Some(StartRefusal::AlreadyRunning));
    // The requirement is that the second process **never exists**, not that it is cleaned up
    // afterwards. A check after the spawn would satisfy the error code and not FR-031c.
    assert_eq!(
        runner.spawns().len(),
        1,
        "a second process was created under a live identity"
    );
}

#[test]
fn an_unregistered_workspace_is_refused() {
    let (fs, roots, runner, mut tasks) = setup();
    let mut p = params(None);
    p.workspace_id = WorkspaceId("never".into());
    let result = start_task(&p, &roots, fs.as_ref(), &runner, &mut tasks);
    assert_eq!(result.err(), Some(StartRefusal::NotRegistered));
    assert!(runner.spawns().is_empty());
}

#[test]
fn an_omitted_size_becomes_eighty_by_twenty_four() {
    let (fs, roots, runner, mut tasks) = setup();
    runner.script(Script::of(vec![]));
    start_task(&params(None), &roots, fs.as_ref(), &runner, &mut tasks).expect("start");

    // The default is applied in the use case, so the port is handed a size somebody chose. The
    // kernel's own default for a fresh pseudo-terminal is 0 x 0 -- a size no display has, and
    // the one value `resizePty` refuses.
    assert_eq!(
        runner.spawns()[0].shape,
        Shape::Pty {
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS
        }
    );
}

#[test]
fn a_stated_size_is_passed_through_unchanged() {
    let (fs, roots, runner, mut tasks) = setup();
    runner.script(Script::of(vec![]));
    let mut p = params(None);
    p.cols = Some(120);
    p.rows = Some(40);
    start_task(&p, &roots, fs.as_ref(), &runner, &mut tasks).expect("start");
    assert_eq!(
        runner.spawns()[0].shape,
        Shape::Pty {
            cols: 120,
            rows: 40
        }
    );
}

#[test]
fn the_callers_environment_is_merged_over_the_engines_rather_than_replacing_it() {
    let (fs, roots, runner, mut tasks) = setup();
    runner.script(Script::of(vec![]));
    let mut p = params(None);
    let mut env = BTreeMap::new();
    env.insert("APEX_TEST_MARKER".to_string(), "set-by-caller".to_string());
    p.env = Some(env);
    start_task(&p, &roots, fs.as_ref(), &runner, &mut tasks).expect("start");

    let spawned: BTreeMap<String, String> = runner.spawns()[0].env.iter().cloned().collect();
    assert_eq!(
        spawned.get("APEX_TEST_MARKER").map(String::as_str),
        Some("set-by-caller")
    );
    // The half that matters. Replacement is the obvious reading of a bare parameter, and a task
    // started with one variable set would lose PATH and fail for a reason that looks nothing
    // like its cause.
    assert!(
        spawned.contains_key("PATH"),
        "PATH did not survive the merge, so the environment was replaced rather than merged"
    );
}

#[test]
fn a_caller_can_override_an_inherited_variable() {
    let (fs, roots, runner, mut tasks) = setup();
    runner.script(Script::of(vec![]));
    let mut p = params(None);
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), "/only/this".to_string());
    p.env = Some(env);
    start_task(&p, &roots, fs.as_ref(), &runner, &mut tasks).expect("start");

    let spawned: BTreeMap<String, String> = runner.spawns()[0].env.iter().cloned().collect();
    // "Over", not "beside": a caller that names a variable the engine also has wins.
    assert_eq!(spawned.get("PATH").map(String::as_str), Some("/only/this"));
}
