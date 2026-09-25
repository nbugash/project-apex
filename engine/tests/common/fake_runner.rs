//! `TaskRunner` with no process at all.
//!
//! A contract obligation rather than a test helper, for the reason `FakeFileSystem` is: the real
//! adapter is Linux-only by construction, and a real pseudo-terminal cannot be driven
//! deterministically. A build that outruns a link, a process that exits with bytes still
//! buffered, and a kernel refusing a limit are cases a real process produces rarely and a test
//! must produce every run.
//!
//! See `contracts/runner-port.md`, *The fake*, for what this owes.

#![allow(dead_code)]

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

use apex_engine::application::ports::clock::Millis;
use apex_engine::application::ports::task_runner::{
    ControlError, Exit, ReadOutcome, ResourceLimits, SpawnFailure, SpawnRequest, SpawnedTask,
    TaskControl, TaskOutput, TaskRunner,
};
use apex_engine::domain::task::{Shape, Stream, TaskSignal};
use apex_protocol::wire::Pid;

/// One step of a scripted output sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Bytes on a stream.
    Bytes(Stream, Vec<u8>),
    /// A read that returns with nothing, as a real one does when the timeout expires.
    Idle,
    /// A read that blocks until the test releases it.
    ///
    /// This is how T13 is asserted rather than assumed: a signal has to be able to land while a
    /// read is outstanding, which is the whole reason `TaskControl` is `Sync` and separate from
    /// `TaskOutput`.
    Block,
    /// Bytes scripted **after** the process is marked exited.
    ///
    /// The spec's *output arriving after the process has exited* edge case, and unreachable on
    /// demand with a real process. `Ended` must still come after these, or T4 and FR-022 are
    /// assumed rather than tested.
    BytesAfterExit(Stream, Vec<u8>),
}

/// What one spawned task will do.
#[derive(Debug, Clone, Default)]
pub struct Script {
    pub steps: Vec<Step>,
    pub exit: Option<Exit>,
    /// Pids in this task's process group, the leader first. A signal reaches all of them.
    pub group: Vec<Pid>,
}

impl Script {
    pub fn of(steps: Vec<Step>) -> Self {
        Self {
            steps,
            exit: Some(Exit::Code(0)),
            group: Vec::new(),
        }
    }

    /// A 4 MiB run with no newline anywhere in it. SC-005 as a unit test.
    pub fn four_mib_no_newline() -> Self {
        Self::of(vec![Step::Bytes(
            Stream::Stdout,
            vec![b'x'; 4 * 1024 * 1024],
        )])
    }

    /// A read that yields nothing, forever. Exercises the retention bound and the decision to
    /// stop reading without a process capable of outrunning a link.
    pub fn silent_forever() -> Self {
        Self {
            steps: vec![Step::Idle; 1_000_000],
            exit: None,
            group: Vec::new(),
        }
    }

    pub fn exiting_with(mut self, exit: Exit) -> Self {
        self.exit = Some(exit);
        self
    }

    /// A process group: the leader and whatever it spawned. A fake that recorded only that
    /// `signal` was called would test half of FR-018.
    pub fn with_group(mut self, pids: &[i32]) -> Self {
        self.group = pids.iter().map(|p| Pid(*p)).collect();
        self
    }
}

/// What a spawn was asked for. The environment is recorded so a test can assert the merge, and
/// is never rendered anywhere (FR-005a).
#[derive(Debug, Clone)]
pub struct SpawnRecord {
    pub command: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub shape: Shape,
    pub limits: ResourceLimits,
}

#[derive(Default)]
struct Shared {
    stdin: Mutex<Vec<u8>>,
    resizes: Mutex<Vec<(u16, u16)>>,
    signals: Mutex<Vec<TaskSignal>>,
    /// Every pid that actually received a signal, which is what makes "to the group" assertable.
    signalled: Mutex<HashSet<i32>>,
    group: Mutex<Vec<Pid>>,
    exit: Mutex<Option<Exit>>,
    exited: Mutex<bool>,
    /// A blocked `read` waits here until `release` is called.
    /// Permits for . Counted, not a latch: see .
    gate: Mutex<usize>,
    opened: Condvar,
}

/// The controlling half of a fake task.
pub struct FakeControl {
    shared: Arc<Shared>,
}

impl FakeControl {
    pub fn stdin(&self) -> Vec<u8> {
        self.shared.stdin.lock().expect("stdin").clone()
    }
    pub fn resizes(&self) -> Vec<(u16, u16)> {
        self.shared.resizes.lock().expect("resizes").clone()
    }
    pub fn signals(&self) -> Vec<TaskSignal> {
        self.shared.signals.lock().expect("signals").clone()
    }
    /// Which pids received a signal. A group signal reaches every member; a pid signal would
    /// reach only the leader, which is the difference SC-012 and SC-027 measure.
    pub fn signalled_pids(&self) -> Vec<i32> {
        let mut v: Vec<i32> = self
            .shared
            .signalled
            .lock()
            .expect("signalled")
            .iter()
            .copied()
            .collect();
        v.sort_unstable();
        v
    }
    /// Whether the scripted process has been marked exited.
    ///
    /// Exposed because the window FR-019 is about cannot be observed from the outside: the reader
    /// drains only when a read returns, so a reader blocked mid-drain flushes nothing and there
    /// is no frame to wait for. A test that waited on output would wait forever for bytes the
    /// blocked reader is holding.
    pub fn has_exited(&self) -> bool {
        *self.shared.exited.lock().expect("exited")
    }

    /// Let **one** blocked `read` return.
    ///
    /// A permit rather than a latch. The first version set a flag, so one release let every later
    /// `Block` through and a script could be gated exactly once -- which is not enough to stage
    /// "before the client left", "while it was away" and "after it came back", and those three
    /// moments are the whole of what a reattachment test is about.
    pub fn release_read(&self) {
        *self.shared.gate.lock().expect("gate") += 1;
        self.shared.opened.notify_all();
    }
}

impl TaskControl for FakeControl {
    fn write_stdin(&self, data: &[u8]) -> Result<(), ControlError> {
        if *self.shared.exited.lock().expect("exited") {
            return Err(ControlError::Gone);
        }
        self.shared
            .stdin
            .lock()
            .expect("stdin")
            .extend_from_slice(data);
        Ok(())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), ControlError> {
        self.shared
            .resizes
            .lock()
            .expect("resizes")
            .push((cols, rows));
        Ok(())
    }

    fn signal(&self, signal: TaskSignal) -> Result<(), ControlError> {
        // The same guard the real control has, and modelled rather than stubbed: a fake that
        // signalled a task it knows has exited would let the engine do so too and report nothing.
        if *self.shared.exited.lock().expect("exited") {
            return Err(ControlError::Gone);
        }
        self.shared.signals.lock().expect("signals").push(signal);
        // To the **group**. Every member is marked, which is what lets a test tell a group
        // signal from a pid signal -- the distinction FR-018 and SC-027 rest on.
        let group = self.shared.group.lock().expect("group").clone();
        let mut hit = self.shared.signalled.lock().expect("signalled");
        for pid in group {
            hit.insert(pid.0);
        }
        Ok(())
    }

    fn reap(&self) -> Option<Exit> {
        if *self.shared.exited.lock().expect("exited") {
            *self.shared.exit.lock().expect("exit")
        } else {
            None
        }
    }
}

/// The reading half. `Send` and not `Sync`, like the port.
pub struct FakeOutput {
    steps: VecDeque<Step>,
    shared: Arc<Shared>,
    /// The clock a read advances while it waits. Optional, because most tests never look at time.
    clock: Option<Arc<crate::common::fake_clock::FakeClock>>,
    /// Set once the scripted steps are spent, so `Ended` is returned exactly once thereafter.
    done: bool,
}

impl TaskOutput for FakeOutput {
    fn read(&mut self, _timeout: Millis, out: &mut Vec<u8>) -> ReadOutcome {
        loop {
            let Some(step) = self.steps.pop_front() else {
                if !self.done {
                    self.done = true;
                    *self.shared.exited.lock().expect("exited") = true;
                }
                return ReadOutcome::Ended;
            };
            match step {
                Step::Bytes(stream, bytes) => {
                    let len = bytes.len();
                    out.extend_from_slice(&bytes);
                    return ReadOutcome::Bytes { stream, len };
                }
                Step::BytesAfterExit(stream, bytes) => {
                    // The process is over; its bytes are not. `Ended` still comes after these.
                    *self.shared.exited.lock().expect("exited") = true;
                    let len = bytes.len();
                    out.extend_from_slice(&bytes);
                    return ReadOutcome::Bytes { stream, len };
                }
                Step::Idle => {
                    // **Time passes, as it does in a real read.**
                    //
                    // A read that returns nothing has waited out its timeout, and the chunker's
                    // time bound is measured against that same clock. Returning instantly makes
                    // the reader spin: it consumes every scripted idle within one tick, the bound
                    // never expires, and bytes written before an idle never reach the wire --
                    // which is a property of the fake, not of the engine, and it hid the ordering
                    // this suite exists to check.
                    //
                    // Advancing rather than sleeping keeps a million-step `silent_forever` free.
                    if let Some(clock) = &self.clock {
                        clock.advance(_timeout);
                    }
                    return ReadOutcome::Idle;
                }
                Step::Block => {
                    let mut permits = self.shared.gate.lock().expect("gate");
                    while *permits == 0 {
                        permits = self.shared.opened.wait(permits).expect("gate wait");
                    }
                    // Spent, so the next `Block` waits for its own release.
                    *permits -= 1;
                    // Released: carry on to the next step rather than returning nothing.
                    continue;
                }
            }
        }
    }
}

/// A `TaskRunner` that spawns nothing.
#[derive(Default)]
pub struct FakeRunner {
    /// Handed to each `FakeOutput`, so an idle read advances time the way a real one does.
    clock: Mutex<Option<Arc<crate::common::fake_clock::FakeClock>>>,
    scripts: Mutex<VecDeque<Script>>,
    failures: Mutex<VecDeque<SpawnFailure>>,
    spawned: Mutex<Vec<SpawnRecord>>,
    controls: Mutex<Vec<Arc<FakeControl>>>,
    next_pid: Mutex<i32>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self {
            next_pid: Mutex::new(1000),
            ..Default::default()
        }
    }

    /// Let an idle read advance this clock by the timeout it was given.
    ///
    /// Without it a scripted idle costs no time, the chunker's bound never expires, and anything
    /// written before that idle waits in the chunker forever.
    pub fn driving(self: &Arc<Self>, clock: Arc<crate::common::fake_clock::FakeClock>) {
        *self.clock.lock().expect("clock") = Some(clock);
    }

    /// Queue what the next spawn will do.
    pub fn script(&self, script: Script) {
        self.scripts.lock().expect("scripts").push_back(script);
    }

    /// Make the next spawn fail. Every variant is reachable, because FR-004 and SC-015 live
    /// entirely in the failure path and a fake that cannot fail tests only the happy one.
    pub fn fail_next(&self, failure: SpawnFailure) {
        self.failures.lock().expect("failures").push_back(failure);
    }

    pub fn spawns(&self) -> Vec<SpawnRecord> {
        self.spawned.lock().expect("spawned").clone()
    }

    /// The control half of the nth spawned task, for assertions.
    pub fn control(&self, index: usize) -> Arc<FakeControl> {
        Arc::clone(&self.controls.lock().expect("controls")[index])
    }
}

impl TaskRunner for FakeRunner {
    fn spawn(&self, request: &SpawnRequest<'_>) -> Result<SpawnedTask, SpawnFailure> {
        if let Some(failure) = self.failures.lock().expect("failures").pop_front() {
            return Err(failure);
        }

        self.spawned.lock().expect("spawned").push(SpawnRecord {
            command: request.command.to_vec(),
            cwd: request.cwd.as_path().display().to_string(),
            env: request.env.to_vec(),
            shape: request.shape,
            limits: request.limits,
        });

        let script = self
            .scripts
            .lock()
            .expect("scripts")
            .pop_front()
            .unwrap_or_default();

        let mut pid_slot = self.next_pid.lock().expect("pid");
        *pid_slot += 1;
        let pid = Pid(*pid_slot);
        drop(pid_slot);

        // With a pseudo-terminal there is one device, so nothing can be tagged stderr. The fake
        // enforces it rather than trusting the script (A-TASKSTREAM, T3, SC-028).
        let steps: VecDeque<Step> = script
            .steps
            .iter()
            .cloned()
            .map(|s| match (request.shape, s) {
                (Shape::Pty { .. }, Step::Bytes(_, b)) => Step::Bytes(Stream::Stdout, b),
                (Shape::Pty { .. }, Step::BytesAfterExit(_, b)) => {
                    Step::BytesAfterExit(Stream::Stdout, b)
                }
                (_, other) => other,
            })
            .collect();

        let mut group = script.group.clone();
        if group.is_empty() {
            group.push(pid);
        }

        let shared = Arc::new(Shared {
            exit: Mutex::new(script.exit),
            group: Mutex::new(group),
            ..Default::default()
        });

        let control = Arc::new(FakeControl {
            shared: Arc::clone(&shared),
        });
        self.controls
            .lock()
            .expect("controls")
            .push(Arc::clone(&control));

        Ok(SpawnedTask {
            pid,
            output: Box::new(FakeOutput {
                steps,
                shared,
                clock: self.clock.lock().expect("clock").clone(),
                done: false,
            }),
            control,
        })
    }
}
