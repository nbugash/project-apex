//! SC-012 and SC-027: stopping a task stops everything it started.
//!
//! **Two assertions, deliberately.** SC-012 counts the named process and its direct children;
//! SC-027 counts arbitrary depth. An implementation that signals the pid rather than the group
//! satisfies neither, but one that signals the pid *and* its immediate children satisfies the
//! first and fails the second -- which is why the criteria were narrowed apart and why
//! `fixture_tree` builds **three** levels. Against a two-level fixture the pair would be
//! measuring one thing twice.
//!
//! No process in the tree installs a handler. Children do not inherit a signal sent to their
//! parent, so outliving it is the default: only something that signals the process **group**
//! reaches all three, which is what makes a passing run mean anything.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::TaskSignal;
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::frames::{delivered_bytes, frames_of, Sink};
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

/// Is `pid` a running process? A zombie is not: `/proc/<pid>` survives until the parent reaps,
/// so the directory alone answers "has this been reaped" rather than "is this still running".
fn running(pid: i32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(after) = stat.rsplit_once(')') else {
        return false;
    };
    !matches!(after.1.split_whitespace().next(), Some("Z") | None)
}

struct Tree {
    service: TaskService,
    sink: Sink,
    id: TaskId,
}

impl Drop for Tree {
    /// Kill every level, not only the one the service knows about.
    ///
    /// The service signals the **group**, which is the thing under test -- so a teardown that
    /// relied on it would be relying on the property the mutation removes. With only the leader
    /// signalled, the surviving descendants keep the pseudo-terminal open, the reader never sees
    /// end of file, and `close` waits on it forever: quickstart §10's second mutation hung for
    /// ten minutes instead of failing in one second, which is a failure nobody can read.
    fn drop(&mut self) {
        if let Some(control) = self.service.control(&self.id) {
            let _ = control.signal(TaskSignal::Kill);
        }
        for pid in self.pids() {
            // SAFETY: `kill` with a valid signal on a pid that may no longer exist is defined;
            // it answers ESRCH, which is the ordinary case here.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        self.service.close();
    }
}

impl Tree {
    fn text(&self) -> String {
        String::from_utf8_lossy(&delivered_bytes(&frames_of(&self.sink))).into_owned()
    }

    /// The pid each level printed, level 0 first.
    ///
    /// Read from the processes themselves rather than walked from `/proc`: a walk would have to
    /// decide what counts as a descendant, which is the very thing under test.
    fn pids(&self) -> Vec<i32> {
        let text = self.text();
        let mut found = Vec::new();
        for level in 0..3 {
            let marker = format!("LEVEL {level} pid=");
            if let Some(at) = text.find(&marker) {
                let rest = &text[at + marker.len()..];
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(pid) = digits.parse::<i32>() {
                    found.push(pid);
                }
            }
        }
        found
    }
}

fn start_tree(pty: bool) -> Tree {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner.clone());

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let id = TaskId("tree".into());
    let params = RunTaskParams {
        workspace_id: WorkspaceId("ws1".into()),
        task_id: id.clone(),
        command: vec![fixture("fixture_tree")],
        cwd: None,
        env: Some(
            [(
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_default(),
            )]
            .into_iter()
            .collect(),
        ),
        pty,
        cols: None,
        rows: None,
    };
    service.run(&params, &roots, fs.as_ref()).expect("run");

    let tree = Tree { service, sink, id };
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && tree.pids().len() < 3 {
        std::thread::sleep(Duration::from_millis(20));
    }
    let pids = tree.pids();
    assert_eq!(
        pids.len(),
        3,
        "the fixture never reached three levels; nothing can be concluded: {}",
        tree.text()
    );
    tree
}

/// Stop the tree and assert nothing survives, at either depth.
fn assert_no_survivors(pty: bool) {
    let tree = start_tree(pty);
    let pids = tree.pids();
    // All three alive before the stop, or a later "zero survivors" would be true for the wrong
    // reason -- a tree that never got built has nothing left running either.
    for (level, pid) in pids.iter().enumerate() {
        assert!(
            running(*pid),
            "level {level} (pid {pid}) was not running before the stop"
        );
    }

    tree.service
        .control(&tree.id)
        .expect("live")
        .signal(TaskSignal::Kill)
        .expect("kill");

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && pids.iter().any(|p| running(*p)) {
        std::thread::sleep(Duration::from_millis(20));
    }

    // SC-012: the named process and its direct child.
    let shallow: Vec<i32> = pids
        .iter()
        .take(2)
        .copied()
        .filter(|p| running(*p))
        .collect();
    assert!(
        shallow.is_empty(),
        "SC-012: {} process(es) at the first two levels survived: {shallow:?}",
        shallow.len()
    );

    // SC-027: the third generation. This is the assertion an implementation that signals the
    // named process and its immediate children still fails.
    let deep: Vec<i32> = pids
        .iter()
        .skip(2)
        .copied()
        .filter(|p| running(*p))
        .collect();
    assert!(
        deep.is_empty(),
        "SC-027: the grandchild survived: {deep:?}. The signal reached the process rather than \
         the group."
    );
}

#[test]
fn stopping_a_task_with_pipes_leaves_no_survivor_at_any_depth() {
    // **This is the discriminating case.** With pipes there is no controlling terminal, so
    // nothing else reaches the descendants: the only mechanism that can is a signal to the
    // process group.
    //
    // With a terminal there is a second mechanism, and it hides the first. When a session leader
    // dies the kernel sends SIGHUP to the foreground process group of its controlling terminal,
    // which kills the tree whether or not the engine signalled the group -- so the terminal case
    // below passes even for an implementation that signals the named pid alone. That was not a
    // guess: the mutation quickstart §10 names, `kill` in place of `killpg`, passed against the
    // terminal case and fails against this one.
    assert_no_survivors(false);
}

#[test]
fn stopping_a_terminal_task_leaves_no_survivor_at_any_depth() {
    // The shape a developer actually runs a build in. Kept although it does not discriminate,
    // because SC-012 and SC-027 are claims about tasks as they are used, and a criterion checked
    // only in the shape that makes it easy to check is a criterion checked somewhere else.
    assert_no_survivors(true);
}

#[test]
fn the_engine_itself_is_not_in_the_group_it_signals() {
    // The other half of signalling a group, and the one whose failure is catastrophic rather
    // than merely wrong: if the child did not get its own session, the group being signalled is
    // the engine's, and a stop would kill the engine and every other task with it.
    let tree = start_tree(true);
    let ours = std::process::id() as i32;
    let pids = tree.pids();
    assert!(
        !pids.contains(&ours),
        "the task tree contains this process, which means it never left our process group"
    );

    tree.service
        .control(&tree.id)
        .expect("live")
        .signal(TaskSignal::Kill)
        .expect("kill");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        running(ours),
        "signalling the task's group killed the engine"
    );
}
