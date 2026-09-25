//! SC-008 and the escalation contract, which are one check in two halves.
//!
//! Driven through the RPC boundary rather than through the use case, because the thing under test
//! is the whole path: the signal named on the wire, the signal the kernel delivers, and -- for a
//! `SIGTERM` -- the second signal an escalation thread sends five seconds later without anybody
//! asking again.
//!
//! `fixture_signals` **catches and keeps running**. That is the fixture's whole point: US2.2
//! asserts an interrupted task is still alive afterwards, which only means anything if it could
//! have died. A fixture that exited on `SIGINT` would satisfy that assertion on any
//! implementation, including one that sends nothing at all.
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

/// Is `pid` still a **running** process? Read from `/proc`, because the question is about the
/// kernel's view and not about anything this process recorded.
///
/// A zombie does not count. `/proc/<pid>` exists until the parent reaps, so the directory alone
/// answers "has this pid been reaped" rather than "is this task still running" -- and reaping
/// happens on the reader thread, which under load can trail the kill by long enough to make an
/// escalation test fail for a process that is already dead. That was a flake in this file, not a
/// missing SIGKILL, and it is the difference between the two that the state field carries.
fn alive(pid: Pid) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid.0)) else {
        return false;
    };
    // `comm` is parenthesised and may itself contain spaces and brackets, so the state is the
    // first field after the **last** `)` rather than the third field of a naive split.
    let Some(after) = stat.rsplit_once(')') else {
        return false;
    };
    !matches!(after.1.split_whitespace().next(), Some("Z") | None)
}

struct Running {
    service: TaskService,
    sink: Sink,
    id: TaskId,
    pid: Pid,
}

impl Running {
    fn text(&self) -> String {
        String::from_utf8_lossy(&delivered_bytes(&frames_of(&self.sink))).into_owned()
    }

    /// Wait until the fixture has said something containing `needle`, or give up.
    fn wait_for(&self, needle: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn signal(&self, signal: TaskSignal) {
        let control = self.service.control(&self.id).expect("the task is live");
        control.signal(signal).expect("signal");
        if let apex_engine::application::use_cases::task::StopPlan {
            escalate_at: Some(at),
            ..
        } = apex_engine::application::use_cases::task::stop_task(
            &self.id,
            signal,
            self.service.now(),
        )
        .expect("planned")
        {
            self.service
                .escalations()
                .register(self.id.clone(), self.pid, &control, at);
        }
    }
}

impl Drop for Running {
    /// Kill and close on the way out, whatever happened.
    ///
    /// A failing assertion unwinds past the explicit teardown, leaving `fixture_signals` running
    /// and its reader thread inside `TaskService`, which the test binary then waits on forever.
    /// A test that hangs on failure is worse than one that fails: it reports nothing, and it
    /// holds the whole suite. This was not hypothetical -- the mutation that proved the pty fix
    /// hung for minutes instead of failing in five seconds.
    fn drop(&mut self) {
        if let Some(control) = self.service.control(&self.id) {
            let _ = control.signal(TaskSignal::Kill);
        }
        self.service.close();
    }
}

fn start(shape: Shape) -> Running {
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
    let id = TaskId("sig".into());
    service.adopt(id.clone(), pid, task.control, task.output);

    let running = Running {
        service,
        sink,
        id,
        pid,
    };
    assert!(
        running.wait_for("READY", Duration::from_secs(10)),
        "the fixture never started: {}",
        running.text()
    );
    running
}

#[test]
fn an_interrupt_arrives_as_a_signal_and_does_not_escalate() {
    // US2.2, all three halves. The signal is delivered; **zero** 0x03 bytes appear in what the
    // task read, because a terminal turns the byte into a signal rather than passing it through;
    // and the task is still running afterwards, because SIGINT deliberately has no follow-up.
    let running = start(Shape::Pty { cols: 80, rows: 24 });
    running.signal(TaskSignal::Int);

    assert!(
        running.wait_for("CAUGHT signal=2", Duration::from_secs(5)),
        "the interrupt never arrived as a signal: {}",
        running.text()
    );

    // Still alive after the grace period a SIGTERM would have had. A program that legitimately
    // handles an interrupt must not be killed for having handled it.
    std::thread::sleep(Duration::from_millis(5_500));
    assert!(
        alive(running.pid),
        "the task was killed although SIGINT does not escalate"
    );
}

#[test]
fn a_ctrl_c_byte_becomes_a_signal_under_a_terminal_and_never_arrives_as_data() {
    // US2.2's other half, and the one a panel has to branch on. With a terminal the line
    // discipline turns a written 0x03 into SIGINT for the foreground process group, so the
    // program is interrupted and **never sees the byte**. Asserting only that SIGINT arrived
    // would pass for an engine that sent a signal *and* forwarded the byte, which is a task
    // receiving an interrupt it also has to parse out of its input.
    let running = start(Shape::Pty { cols: 80, rows: 24 });
    let control = running.service.control(&running.id).expect("live");
    control.write_stdin(&[0x03]).expect("write");

    assert!(
        running.wait_for("CAUGHT signal=2", Duration::from_secs(5)),
        "0x03 through a terminal did not become SIGINT: {}",
        running.text()
    );
    // Give any stray data time to arrive before concluding that none did. Asserting a negative
    // immediately would pass simply by being early.
    std::thread::sleep(Duration::from_millis(500));
    let text = running.text();
    assert!(
        !text.contains("etx=1"),
        "the interrupt byte also arrived as data: {text}"
    );
}

#[test]
fn the_same_byte_is_plain_data_to_a_task_without_a_terminal() {
    // The contrast that makes the case above mean something. With pipes there is no line
    // discipline, so 0x03 is a byte and nothing else -- which is why an interrupt for a
    // `pty: false` task has to be `execution/terminate` and not a keystroke.
    let running = start(Shape::Pipes);
    let control = running.service.control(&running.id).expect("live");
    control.write_stdin(&[0x03]).expect("write");

    assert!(
        running.wait_for("etx=1", Duration::from_secs(5)),
        "0x03 through pipes did not arrive as data: {}",
        running.text()
    );
    assert!(
        !running.text().contains("CAUGHT signal=2"),
        "pipes delivered a signal, which means a line discipline is involved: {}",
        running.text()
    );
}

#[test]
fn a_term_is_caught_and_survived_and_then_killed_after_the_grace_period() {
    // US2.6, and both halves are required. An assertion only that the task is gone *eventually*
    // passes an implementation with no waiting period at all -- which would kill a program in
    // the middle of the cleanup SIGTERM exists to let it do.
    let running = start(Shape::Pty { cols: 80, rows: 24 });
    running.signal(TaskSignal::Term);

    assert!(
        running.wait_for("CAUGHT signal=15", Duration::from_secs(5)),
        "the terminate never arrived as a signal: {}",
        running.text()
    );

    // Shortly before the deadline: still there, because it caught the signal and kept running.
    std::thread::sleep(Duration::from_millis(3_000));
    assert!(
        alive(running.pid),
        "the task was killed before its grace period had elapsed"
    );

    // Shortly after: gone, without anybody asking a second time.
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline && alive(running.pid) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !alive(running.pid),
        "the task survived its escalation; no SIGKILL followed the SIGTERM"
    );
}
