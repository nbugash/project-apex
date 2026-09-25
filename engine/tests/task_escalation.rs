//! The escalation thread's half of the stop (T032d).
//!
//! `StopTask` sends `SIGTERM` and registers a deadline; this is what happens after. The use
//! case's half -- that it registers exactly one deadline and does not sleep -- is T074's, and
//! the two are deliberately separate: a test that drove both from the use case could only pass
//! against an inline wait, which is the shape this design replaced.

mod common;

use apex_engine::adapters::outbound::task_threads::Escalations;
use apex_engine::application::ports::task_runner::{ControlError, TaskControl};
use apex_engine::domain::task::TaskSignal;
use apex_protocol::wire::{Pid, TaskId};
use common::fake_clock::FakeClock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Records what it was asked to send. Enough of a `TaskControl` to answer "was it killed?".
#[derive(Default)]
struct Recording {
    signals: Mutex<Vec<TaskSignal>>,
    /// Shared with the test, and deliberately **not** owned by this struct.
    ///
    /// The reuse test has to drop every strong reference to the control handle -- that is what
    /// a release does -- and then still observe whether a kill arrived. A counter living inside
    /// the handle is dropped along with it, so the test would be asserting on evidence it had
    /// just destroyed.
    kills: Arc<AtomicUsize>,
}

impl Recording {
    fn with_counter(kills: Arc<AtomicUsize>) -> Self {
        Self {
            signals: Mutex::new(Vec::new()),
            kills,
        }
    }
    fn kills(&self) -> usize {
        self.kills.load(Ordering::SeqCst)
    }
    fn signals(&self) -> Vec<TaskSignal> {
        self.signals.lock().expect("signals").clone()
    }
}

impl TaskControl for Recording {
    fn write_stdin(&self, _data: &[u8]) -> Result<(), ControlError> {
        Ok(())
    }
    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), ControlError> {
        Ok(())
    }
    fn signal(&self, signal: TaskSignal) -> Result<(), ControlError> {
        self.signals.lock().expect("signals").push(signal);
        if signal == TaskSignal::Kill {
            self.kills.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
    fn reap(&self) -> Option<apex_engine::application::ports::task_runner::Exit> {
        None
    }
}

/// Wait for a condition, bounded. The thread is concurrent, so asserting immediately after
/// advancing the clock is a race: the assertion would be about scheduling rather than about the
/// deadline. Waiting for the effect is the handshake.
fn until(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out waiting for: {what}");
}

/// Give the thread a chance to do the wrong thing. Used only for negative assertions, where
/// there is no effect to wait for -- "nothing happened" has no edge to synchronise on.
fn settle() {
    std::thread::sleep(Duration::from_millis(120));
}

fn setup() -> (
    Arc<FakeClock>,
    Escalations,
    Arc<Recording>,
    Arc<dyn TaskControl>,
) {
    let clock = Arc::new(FakeClock::new());
    let esc = Escalations::spawn(Arc::clone(&clock) as Arc<_>);
    let rec = Arc::new(Recording::with_counter(Arc::new(AtomicUsize::new(0))));
    let control: Arc<dyn TaskControl> = rec.clone();
    (clock, esc, rec, control)
}

#[test]
fn nothing_is_killed_before_its_deadline() {
    let (clock, esc, rec, control) = setup();
    esc.register(TaskId("build".into()), Pid(1), &control, 5_000);

    clock.set(4_999);
    settle();
    assert_eq!(rec.kills(), 0, "4 999 ms is before 5 000 ms");
}

#[test]
fn the_deadline_boundary_is_inclusive() {
    let (clock, esc, rec, control) = setup();
    esc.register(TaskId("build".into()), Pid(1), &control, 5_000);

    clock.set(5_000);
    until("the kill", || rec.kills() == 1);
    // Exactly one, not at least one: a thread that re-fired a deadline it had already taken
    // would send a second SIGKILL to a pid that may by then belong to something else.
    settle();
    assert_eq!(rec.kills(), 1);
    assert_eq!(rec.signals(), vec![TaskSignal::Kill]);
}

#[test]
fn a_task_that_ends_before_its_deadline_is_not_killed() {
    let (clock, esc, rec, control) = setup();
    esc.register(TaskId("build".into()), Pid(1), &control, 5_000);

    // The task ended on its own, so the use case cancels.
    esc.cancel(&TaskId("build".into()));
    clock.set(10_000);
    settle();
    assert_eq!(rec.kills(), 0, "a cancelled deadline must not fire");
}

/// A released task's deadline must fire into nothing.
///
/// The sequence this guards: terminate `build`, it exits on its own a second later, the id is
/// released, the client re-runs `build` in the same panel, and five seconds after the *first*
/// stop the escalation comes due. A deadline that resolved its target by identity would find
/// the new task and kill it. This one holds a `Weak` to the specific handle it was registered
/// against, so there is nothing left to reach.
#[test]
fn a_released_tasks_deadline_reaches_nothing() {
    let clock = Arc::new(FakeClock::new());
    let esc = Escalations::spawn(Arc::clone(&clock) as Arc<_>);

    // The counter is the test's, not the handle's, so dropping the handle does not drop the
    // evidence.
    let kills = Arc::new(AtomicUsize::new(0));

    {
        let control: Arc<dyn TaskControl> = Arc::new(Recording::with_counter(Arc::clone(&kills)));
        esc.register(TaskId("build".into()), Pid(1), &control, 5_000);
        // The task ended and was released: the engine's last strong reference goes here.
    }

    clock.set(5_000);
    settle();

    assert_eq!(
        kills.load(Ordering::SeqCst),
        0,
        "a deadline whose task was released must not signal anything"
    );
}

#[test]
fn closing_releases_the_thread_rather_than_hanging() {
    let clock = Arc::new(FakeClock::new());
    let esc = Escalations::spawn(Arc::clone(&clock) as Arc<_>);
    let rec = Arc::new(Recording::with_counter(Arc::new(AtomicUsize::new(0))));
    let control: Arc<dyn TaskControl> = rec.clone();
    esc.register(TaskId("build".into()), Pid(1), &control, u64::MAX);

    // A thread parked on a deadline that will never arrive. If shutdown could not interrupt it,
    // this join would hang -- which is a test that looks slow rather than one that fails.
    let done = Arc::new(AtomicUsize::new(0));
    let flag = Arc::clone(&done);
    let joiner = std::thread::spawn(move || {
        esc.close();
        flag.store(1, Ordering::SeqCst);
    });
    until("close to return", || done.load(Ordering::SeqCst) == 1);
    joiner.join().expect("join");
    assert_eq!(rec.kills(), 0, "closing must not fire pending deadlines");
}

#[test]
fn an_empty_set_costs_nothing_and_still_accepts_work() {
    let (clock, esc, rec, control) = setup();
    // Idle first: the thread is parked on an unbounded wait with nothing pending.
    settle();
    assert_eq!(esc.pending(), 0);

    // Registering must wake it, or the first stop of a run parks behind a thread nothing woke.
    esc.register(TaskId("late".into()), Pid(7), &control, 100);
    clock.set(100);
    until("the late kill", || rec.kills() == 1);
}
