//! The engine's escalation thread.
//!
//! One thread, not one per stop. `StopTask` sends `SIGTERM` and **registers a deadline**; this
//! thread waits and sends `SIGKILL` to the process group of anything still alive when the
//! deadline passes. The use case never sleeps, because it runs on the engine's single dispatch
//! thread -- the same thread that reads the client's stdin -- and five seconds there is five
//! seconds in which no keystroke is so much as read (FR-012, §1.4).
//!
//! The five-second *policy* stays in the use case, which is what keeps Principle VIII's line
//! honest: this thread owns the waiting and the second signal, and decides nothing.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

use apex_protocol::wire::{Pid, TaskId};

use crate::application::ports::clock::{Clock, Millis};
use crate::application::ports::task_runner::TaskControl;
use crate::domain::task::TaskSignal;

/// How long the thread waits between checks while a deadline is pending.
///
/// **Not a behavioural bound and not a plan quantity.** The five seconds a developer is promised
/// is plan.md's; this only bounds how *late* a kill may be, and 25 ms is immaterial against it.
/// While no deadline is pending the thread waits unbounded and wakes not at all, so an idle
/// engine pays nothing.
const POLL: Duration = Duration::from_millis(25);

/// One task's pending kill.
struct Entry {
    at: Millis,
    /// The process this deadline was registered against.
    ///
    /// A **`Weak`**, and this is the whole of the identity problem. A deadline keyed by task id
    /// alone kills whatever holds that id when it fires -- and re-running `build` in the same
    /// panel is what a developer does all day. The sequence that bites: terminate `build`, it
    /// exits on its own a second later, the id is released, the client starts a new `build`,
    /// and five seconds after the first stop the new task is killed. The `Weak` cannot be
    /// upgraded once the released task's control handle is dropped, so the kill goes nowhere.
    control: Weak<dyn TaskControl>,
    /// Recorded so a test can assert *which* process would have been signalled.
    pid: Pid,
}

#[derive(Default)]
struct State {
    closed: bool,
    pending: BTreeMap<TaskId, Entry>,
}

struct Inner {
    state: Mutex<State>,
    /// Notified on registration, on cancellation and on close. A thread parked here is released
    /// by shutdown, which is the property a sleep could not offer.
    changed: Condvar,
    clock: Arc<dyn Clock>,
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Registers deadlines and owns the thread that fires them.
pub struct Escalations {
    inner: Arc<Inner>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Escalations {
    pub fn spawn(clock: Arc<dyn Clock>) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            clock,
        });
        let worker = Arc::clone(&inner);
        let handle = std::thread::spawn(move || run(&worker));
        Self {
            inner,
            handle: Some(handle),
        }
    }

    /// Record that `id` should be killed at `at` unless it ends first.
    pub fn register(&self, id: TaskId, pid: Pid, control: &Arc<dyn TaskControl>, at: Millis) {
        let mut state = self.inner.lock();
        state.pending.insert(
            id,
            Entry {
                at,
                control: Arc::downgrade(control),
                pid,
            },
        );
        drop(state);
        self.inner.changed.notify_all();
    }

    /// Forget a deadline. Called when a task ends on its own, so a `SIGKILL` is not sent to a
    /// process that is already gone -- or, worse, to whatever next holds its identity.
    pub fn cancel(&self, id: &TaskId) {
        let mut state = self.inner.lock();
        state.pending.remove(id);
        drop(state);
        self.inner.changed.notify_all();
    }

    pub fn pending(&self) -> usize {
        self.inner.lock().pending.len()
    }

    /// Stop the thread and wait for it.
    ///
    /// Close **then** join, in that order: joining a thread that has not been told to stop is a
    /// hang. `WatchService` sets the precedent and `client/core`'s send queue tested it as
    /// `closing_releases_a_blocked_writer`.
    pub fn close(&mut self) {
        {
            let mut state = self.inner.lock();
            state.closed = true;
            state.pending.clear();
        }
        self.inner.changed.notify_all();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Escalations {
    fn drop(&mut self) {
        self.close();
    }
}

fn run(inner: &Arc<Inner>) {
    loop {
        let mut state = inner.lock();
        if state.closed {
            return;
        }

        let now = inner.clock.now();
        // `>=`, so a deadline of 5 000 is a kill at exactly 5 000.
        let due: Vec<TaskId> = state
            .pending
            .iter()
            .filter(|(_, e)| now >= e.at)
            .map(|(id, _)| id.clone())
            .collect();

        if due.is_empty() {
            let waited = if state.pending.is_empty() {
                // Nothing pending: wait to be told there is. An idle engine wakes not at all.
                match inner.changed.wait(state) {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                }
            } else {
                // Something pending but not yet due. The clock may be a settable one whose
                // `advance` does not notify this condvar, so this is a bounded wait rather
                // than an indefinite one.
                match inner.changed.wait_timeout(state, POLL) {
                    Ok((g, _)) => g,
                    Err(poisoned) => poisoned.into_inner().0,
                }
            };
            // Released here and re-taken at the top, so a registration made while this thread
            // was waiting is seen on the next pass.
            drop(waited);
            continue;
        }

        let firing: Vec<Entry> = due
            .iter()
            .filter_map(|id| state.pending.remove(id))
            .collect();
        drop(state);

        for entry in firing {
            // The identity check. `None` means the task was released -- it ended on its own and
            // its control handle went with it -- so there is nothing to kill, and in particular
            // nothing belonging to whoever holds that id now.
            if let Some(control) = entry.control.upgrade() {
                let _ = control.signal(TaskSignal::Kill);
            }
            let _ = entry.pid;
        }
    }
}
