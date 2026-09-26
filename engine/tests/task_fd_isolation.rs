//! A task inherits no descriptor it was not given.
//!
//! `fork` copies the whole descriptor table and `exec` keeps everything not marked close-on-exec.
//! Without that mark, a task inherits every descriptor the engine happens to hold at the moment
//! it starts: the pseudo-terminals and pipes of **every other running task**, and whatever else
//! the engine has open.
//!
//! That is a trust-boundary failure before it is anything else (Principle VI). A task handed
//! another task's terminal master can read what that other task is printing and write into it --
//! output a developer will read as coming from their own build. Nothing in the specification
//! grants a task that reach, and §4.8 gives it exactly three descriptors.
//!
//! The property is checked **directly**, by reading the child's own `/proc/<pid>/fd` after it has
//! exec'd, rather than by watching for a downstream symptom. An earlier version of this file
//! inferred it from a task's ending failing to arrive; that version passed with the protection
//! removed, which makes it not a test. The descriptor table is the thing being claimed about, so
//! the descriptor table is what is read.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::task_runner::{ResourceLimits, SpawnRequest, TaskRunner};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::{Shape, TaskSignal};
use apex_protocol::wire::{Pid, TaskId};
use common::frames::{frames_of, Sink};
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

/// Every descriptor a process holds, as `(number, what it points at)`.
///
/// Read after the child has exec'd, which is the only moment that answers the question: before
/// `exec` every descriptor is still there by design, and close-on-exec is exactly the promise
/// that they go away at that instant.
fn open_descriptors(pid: Pid) -> Vec<(i32, String)> {
    let dir = format!("/proc/{}/fd", pid.0);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Some(number) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        let target = std::fs::read_link(entry.path())
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        out.push((number, target));
    }
    out.sort();
    out
}

fn wait_until(mut ready: impl FnMut() -> bool, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

struct Two {
    service: TaskService,
    first: TaskId,
    second: TaskId,
    second_pid: Pid,
}

impl Drop for Two {
    fn drop(&mut self) {
        for id in [self.first.clone(), self.second.clone()] {
            if let Some(control) = self.service.control(&id) {
                let _ = control.signal(TaskSignal::Kill);
            }
        }
        self.service.close();
    }
}

/// Start one task, wait for it to be running, then start a second and return it.
///
/// The order is what makes the test mean anything: the second is forked while the first's
/// descriptors are open, so the second is exactly the process that would inherit them.
fn start_two(shape: Shape) -> Two {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner.clone());

    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_signals")];
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    let spawn = |id: &str| {
        let task = runner
            .spawn(&SpawnRequest {
                command: &command,
                cwd: &cwd,
                env: &env,
                shape,
                limits: ResourceLimits::FIXED,
            })
            .expect("spawn");
        let pid = task.pid;
        let task_id = TaskId(id.into());
        service.adopt(task_id.clone(), pid, task.control, task.output);
        (task_id, pid)
    };

    let (first, _) = spawn("first");
    // "Running" means it has exec'd and reached its own code, which is what makes its
    // descriptors the ones it will keep.
    assert!(
        wait_until(
            || frames_of(&sink)
                .iter()
                .any(|f| f.method.starts_with("execution/on")),
            Duration::from_secs(10)
        ),
        "the first task never started"
    );
    let (second, second_pid) = spawn("second");
    assert!(
        wait_until(
            || open_descriptors(second_pid).len() >= 3,
            Duration::from_secs(10)
        ),
        "the second task never appeared in /proc"
    );
    // The fixture writes on startup; give the exec a moment to have completed so that what is
    // read below is the post-exec table rather than the pre-exec one.
    std::thread::sleep(Duration::from_millis(300));

    Two {
        service,
        first,
        second,
        second_pid,
    }
}

/// A task gets three descriptors and no more.
fn assert_only_standard_descriptors(shape: Shape) {
    let two = start_two(shape);
    let held = open_descriptors(two.second_pid);

    let extra: Vec<&(i32, String)> = held.iter().filter(|(n, _)| *n > 2).collect();
    assert!(
        extra.is_empty(),
        "a task holds {} descriptor(s) beyond its own stdin, stdout and stderr: {extra:?}\n\
         These are the engine's, and with two tasks running they are the other task's terminal.",
        extra.len()
    );
    // And it does have its three: an empty table would satisfy the assertion above for a process
    // that had already died, which would make this pass for the wrong reason.
    assert_eq!(
        held.iter().filter(|(n, _)| *n <= 2).count(),
        3,
        "a task must hold exactly stdin, stdout and stderr: {held:?}"
    );
}

#[test]
fn a_terminal_task_holds_only_its_own_three_descriptors() {
    assert_only_standard_descriptors(Shape::Pty { cols: 80, rows: 24 });
}

#[test]
fn a_pipe_task_holds_only_its_own_three_descriptors() {
    assert_only_standard_descriptors(Shape::Pipes);
}

/// A task's two halves must not close the same descriptor.
///
/// **Concurrency is required to see this, and that is the point.** Dropping a task closes its
/// input descriptor; if both halves hold the same number, it is closed twice. On a single thread
/// that is harmless -- nothing allocates between the two closes, so the second merely fails --
/// which is exactly why it survived every sequential test. The other thread here allocates
/// continuously and *holds* what it opens, so a number freed by the first close is handed to it
/// before the second close arrives and shuts it down.
///
/// It first appeared as `closedir: Bad file descriptor` in an unrelated test under
/// `--test-threads=2`. In the engine the descriptors in that pool are the client's stdin and
/// stdout, the watcher's inotify handle, and every other task's terminal.
#[test]
fn spawning_tasks_does_not_close_another_threads_descriptors() {
    use std::os::fd::AsRawFd;

    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_report_env")];
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    let stop = std::sync::atomic::AtomicBool::new(false);
    let failure: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            // Open and hold, checking everything held is still open. Holding is what matters:
            // a descriptor the victim opened and let go of cannot be stolen from it.
            let mut held: Vec<std::fs::File> = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                match std::fs::File::open("/dev/null") {
                    Ok(f) => held.push(f),
                    Err(e) => {
                        *failure.lock().expect("failure") = Some(format!("open failed: {e}"));
                        return;
                    }
                }
                for f in &held {
                    // `F_GETFD` on a descriptor we still own must succeed. EBADF means somebody
                    // else closed it.
                    // libc rather than nix:  allows nix in exactly one
                    // file, and a test is not it.
                    if unsafe { libc::fcntl(f.as_raw_fd(), libc::F_GETFD) } < 0 {
                        *failure.lock().expect("failure") =
                            Some(format!("fd {} was closed by another thread", f.as_raw_fd()));
                        return;
                    }
                }
                if held.len() > 48 {
                    held.drain(..24);
                }
            }
        });

        let runner = PtyRunner::new();
        for _ in 0..400 {
            let task = runner
                .spawn(&SpawnRequest {
                    command: &command,
                    cwd: &cwd,
                    env: &env,
                    shape: Shape::Pty { cols: 80, rows: 24 },
                    limits: ResourceLimits::FIXED,
                })
                .expect("spawn");
            drop(task.output);
            drop(task.control);
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    let failed = failure.lock().expect("failure").clone();
    assert!(
        failed.is_none(),
        "a descriptor belonging to another thread was closed: {}",
        failed.unwrap_or_default()
    );
}

/// A task inherits nothing the engine never marked, either.
///
/// The failure this was written for: CI showed a child holding two pipes that nothing in
/// `pty_runner` had created, and it could not be reproduced by running the engine's own spawn
/// paths against each other. Both of those facts point the same way -- the descriptor did not
/// come from a creation site the engine controls.
///
/// Marking each descriptor as it is created is a rule that every future creation site has to
/// remember, and it binds nothing outside this crate: a library, a logger, a test harness or the
/// runtime can open a descriptor at any moment on any thread, and a task forked after it keeps
/// it across `exec` for its whole life. §4.8 gives a task three descriptors, so a task holding a
/// fourth is a trust-boundary failure regardless of who opened it (Principle VI).
///
/// This opens one the engine has never heard of and asserts the child does not have it.
#[test]
fn a_task_inherits_nothing_the_engine_never_marked() {
    use std::os::fd::AsRawFd;

    // A **pipe**, and deliberately not a file. The first version of this test opened a file with
    // `File::open` and passed, proving nothing: Rust's standard library sets `O_CLOEXEC` on
    // every file it opens, so the descriptor was already marked and the test agreed with itself.
    // The raw `pipe` syscall sets no such flag, which is why `pty_runner` marks its own pipes by
    // hand -- and why an unmarked one is the honest stand-in for whatever CI's child inherited.
    let (stray_r, stray_w) = nix::unistd::pipe().expect("pipe");
    let stray_fd = stray_r.as_raw_fd();
    let _keep_open = (&stray_r, &stray_w);

    let two = start_two(Shape::Pipes);
    let held = open_descriptors(two.second_pid);

    assert!(
        !held.iter().any(|(n, _)| *n == stray_fd),
        "the task inherited descriptor {stray_fd}, which the engine never created and therefore \
         never marked: {held:?}"
    );
    let extra: Vec<&(i32, String)> = held.iter().filter(|(n, _)| *n > 2).collect();
    assert!(
        extra.is_empty(),
        "a task holds {} descriptor(s) beyond its own three: {extra:?}",
        extra.len()
    );
}
