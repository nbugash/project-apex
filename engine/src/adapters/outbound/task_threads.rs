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
    /// Behind a mutex so `close` takes `&self`. The service shares this inside an `Arc` with
    /// every reader thread, and a `&mut self` shutdown would have to be reached through the one
    /// owner -- which the readers are not.
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
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
            handle: Mutex::new(Some(handle)),
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
    pub fn close(&self) {
        {
            let mut state = self.inner.lock();
            state.closed = true;
            state.pending.clear();
        }
        self.inner.changed.notify_all();
        let taken = self.handle.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(handle) = taken {
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

// ---- TaskService: one reader thread per task (T054, T056) ----

use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::{ExitParams, OutputParams, SignalName};

use crate::adapters::inbound::rpc::encode_notification;
use crate::adapters::outbound::frame_writer::FrameWriter;
use crate::application::output::{Chunker, RetainedOutput};
use crate::application::ports::task_runner::{Exit, ReadOutcome, TaskOutput};
use crate::domain::task::{ExitStatus, OutputChunk, Stream};

/// How long a reader waits when the chunker has nothing pending.
///
/// Not a plan quantity: it bounds only how long an idle reader sits in one `read` before
/// looking again, and a task with bytes waiting is governed by `Chunker::next_deadline`.
const IDLE_READ_MS: Millis = 250;

/// One task's mutable stream state.
///
/// Behind its **own** lock, and this is the whole point. A single map-wide mutex satisfies every
/// document as written and defeats the port's T12 and T13 in one line: a 50 MiB build takes that
/// lock roughly eight hundred times and an idle shell every twenty milliseconds, so a keystroke
/// bound for an unrelated task would queue behind a reader that is busy precisely because its
/// own task is loud.
struct Streams {
    chunker: Chunker,
    retained: RetainedOutput,
}

/// What the service holds for one task.
struct TaskEntry {
    control: Arc<dyn TaskControl>,
    pid: Pid,
    /// No workspace here. `TaskSet` in the domain records which workspace owns a task, and
    /// `workspace/close` drains through it -- a copy in this adapter would be a second source
    /// that can disagree with the first.
    streams: Mutex<Streams>,
}

struct ServiceInner {
    /// The map lock covers **membership only** -- which tasks exist. It is never held while
    /// reading, writing or signalling, so a caller that has an `Arc<TaskEntry>` needs nothing from
    /// it. That is what makes `TaskControl` reachable without the map lock (T13).
    entries: Mutex<BTreeMap<TaskId, Arc<TaskEntry>>>,
    writer: Arc<FrameWriter>,
    codec: FrameCodec,
    clock: Arc<dyn Clock>,
    /// The domain's record of which tasks exist and which workspace owns each.
    ///
    /// Held here rather than on `TaskService` because a reader thread is what learns that a task
    /// has ended, and the ending has to release the identity from the domain's record and from
    /// the machinery together. Two sources released at different moments is two answers to "is
    /// this task still running", and the wire asks that question three ways.
    set: Mutex<crate::domain::task::TaskSet>,
    /// Shared with the reader threads, so a task that ends on its own can cancel the deadline
    /// its own stop registered. Without that the escalation fires into a released identity --
    /// harmless because the entry holds a `Weak`, but it would still be a kill nobody wanted.
    escalations: Escalations,
}

impl ServiceInner {
    fn entry(&self, id: &TaskId) -> Option<Arc<TaskEntry>> {
        let map = match self.entries.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        map.get(id).cloned()
    }
}

/// Runs tasks: one reader thread each, and the escalation thread they share.
pub struct TaskService {
    inner: Arc<ServiceInner>,
    runner: Arc<dyn crate::application::ports::task_runner::TaskRunner>,
    readers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl TaskService {
    pub fn new(
        writer: Arc<FrameWriter>,
        clock: Arc<dyn Clock>,
        runner: Arc<dyn crate::application::ports::task_runner::TaskRunner>,
    ) -> Self {
        Self {
            runner,
            inner: Arc::new(ServiceInner {
                entries: Mutex::new(BTreeMap::new()),
                writer,
                codec: FrameCodec,
                clock: Arc::clone(&clock),
                set: Mutex::new(crate::domain::task::TaskSet::new()),
                escalations: Escalations::spawn(clock),
            }),
            readers: Mutex::new(Vec::new()),
        }
    }

    /// Take ownership of a spawned task and start reading it.
    pub fn adopt(
        &self,
        id: TaskId,
        pid: Pid,
        control: Arc<dyn TaskControl>,
        output: Box<dyn TaskOutput>,
    ) {
        let entry = Arc::new(TaskEntry {
            control,
            pid,
            streams: Mutex::new(Streams {
                chunker: Chunker::new(),
                retained: RetainedOutput::new(),
            }),
        });
        {
            let mut map = match self.inner.entries.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            map.insert(id.clone(), Arc::clone(&entry));
        }
        let inner = Arc::clone(&self.inner);
        let handle = std::thread::spawn(move || read_loop(&inner, &id, &entry, output));
        self.readers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(handle);
    }

    /// Start a task and take ownership of it: the use case decides, this adopts the result.
    ///
    /// The two are one call because they must not be separable. A caller that could start
    /// without adopting would hold a running process nothing reads and nothing can stop, which
    /// is FR-025's abandoned process arrived at through the API rather than through a failure.
    pub fn run(
        &self,
        params: &apex_protocol::wire::RunTaskParams,
        roots: &dyn crate::application::ports::roots::WorkspaceRoots,
        fs: &dyn crate::application::ports::file_system::FileSystem,
    ) -> Result<Pid, crate::application::use_cases::task::StartRefusal> {
        let mut set = self.inner.set.lock().unwrap_or_else(|p| p.into_inner());
        let (pid, spawned) = crate::application::use_cases::task::start_task(
            params,
            roots,
            fs,
            self.runner.as_ref(),
            &mut set,
        )?;
        drop(set);
        self.adopt(params.task_id.clone(), pid, spawned.control, spawned.output);
        Ok(pid)
    }

    /// The control half, without touching the map lock for anything but the lookup.
    pub fn control(&self, id: &TaskId) -> Option<Arc<dyn TaskControl>> {
        self.inner.entry(id).map(|e| Arc::clone(&e.control))
    }

    pub fn pid(&self, id: &TaskId) -> Option<Pid> {
        self.inner.entry(id).map(|e| e.pid)
    }

    /// The shape a task was started with, which is what decides whether a resize means anything.
    ///
    /// Read from the domain's `TaskSet` rather than from the entry, because the set is the single
    /// source for what a task *is* and the entry is the machinery for reaching it.
    pub fn shape(&self, id: &TaskId) -> Option<crate::domain::task::Shape> {
        let set = self.inner.set.lock().unwrap_or_else(|p| p.into_inner());
        set.get(id).map(|task| task.shape)
    }

    /// Now, by the clock this service was built with. The stop path needs it to date a deadline,
    /// and reading it here keeps the dispatch layer from acquiring a clock of its own -- which
    /// would be a second source of time, and a fake one in tests would no longer govern.
    pub fn now(&self) -> Millis {
        self.inner.clock.now()
    }

    pub fn escalations(&self) -> &Escalations {
        &self.inner.escalations
    }

    /// Stop the escalation thread, then wait for the readers.
    ///
    /// Close before join, in that order. Joining a thread that has not been told to stop is a
    /// hang, which is the failure `client/core`'s send queue tested as
    /// `closing_releases_a_blocked_writer`.
    pub fn close(&mut self) {
        self.inner.escalations.close();
        let handles: Vec<_> = self
            .readers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

/// Forget a task, in the machinery and in the domain's record together.
///
/// Both, because they answer the same question and a client can ask it three ways: `terminate`
/// resolves a control, `resizePty` resolves a shape, and `list` reads the record. Releasing one
/// and keeping the other leaves a task that can be resized but not stopped, or listed but not
/// reached.
fn release(inner: &ServiceInner, id: &TaskId) {
    {
        let mut entries = inner.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.remove(id);
    }
    let mut set = inner.set.lock().unwrap_or_else(|p| p.into_inner());
    set.release(id);
}

/// Emit one chunk as `execution/onStdout` or `execution/onStderr`.
///
/// **Bulk**, along with the exit. The two travel the same path written by this one thread in the
/// order it produced them, which is what makes it impossible for the exit to overtake the output
/// FR-022 requires it to follow.
fn emit_chunk(inner: &ServiceInner, id: &TaskId, chunk: &OutputChunk) {
    let method = match chunk.stream {
        Stream::Stdout => "execution/onStdout",
        Stream::Stderr => "execution/onStderr",
    };
    let params = OutputParams {
        task_id: id.clone(),
        // Base64: a JSON string holds text and a task's bytes are not text.
        data: apex_protocol::base64::encode(&chunk.bytes),
    };
    if let Some(frame) = encode_notification(&inner.codec, method, &params) {
        let _ = inner.writer.write_bulk(&frame);
    }
}

fn emit_exit(inner: &ServiceInner, id: &TaskId, status: ExitStatus) {
    let params = match status {
        ExitStatus::Exited { code } => ExitParams {
            task_id: id.clone(),
            exit_code: Some(code),
            signal: None,
        },
        // The name, never the number: numbers differ between platforms and the client is not
        // always on the engine's.
        ExitStatus::Signalled { signal } => ExitParams {
            task_id: id.clone(),
            exit_code: None,
            signal: Some(SignalName::from_number(signal)),
        },
    };
    if let Some(frame) = encode_notification(&inner.codec, "execution/onExit", &params) {
        let _ = inner.writer.write_bulk(&frame);
    }
}

/// Drain whatever the chunker has ready and send it, retaining as it goes.
fn flush(inner: &ServiceInner, id: &TaskId, entry: &TaskEntry, chunks: Vec<OutputChunk>) {
    for chunk in chunks {
        {
            let mut streams = entry.streams.lock().unwrap_or_else(|p| p.into_inner());
            streams.retained.push(chunk.clone());
        }
        emit_chunk(inner, id, &chunk);
        // Released as soon as it is written. Retention holds what an absent client has not seen;
        // a client that is present has seen this the moment the frame went out.
        let mut streams = entry.streams.lock().unwrap_or_else(|p| p.into_inner());
        let _ = streams.retained.drain();
    }
}

fn read_loop(
    inner: &Arc<ServiceInner>,
    id: &TaskId,
    entry: &Arc<TaskEntry>,
    mut output: Box<dyn TaskOutput>,
) {
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    loop {
        // The timeout is the chunker's next due emission, so this thread wakes to flush and for
        // nothing else -- and owns no policy, because the policy is that number.
        let timeout = {
            let streams = entry.streams.lock().unwrap_or_else(|p| p.into_inner());
            match streams.chunker.next_deadline() {
                Some(at) => at.saturating_sub(inner.clock.now()),
                None => IDLE_READ_MS,
            }
        };

        buf.clear();
        let outcome = output.read(timeout, &mut buf);
        let now = inner.clock.now();

        match outcome {
            ReadOutcome::Bytes { stream, len } => {
                let due = {
                    let mut streams = entry.streams.lock().unwrap_or_else(|p| p.into_inner());
                    streams.chunker.accept(stream, &buf[..len], now);
                    streams.chunker.drain_due(now)
                };
                flush(inner, id, entry, due);
            }
            ReadOutcome::Idle => {
                let due = {
                    let mut streams = entry.streams.lock().unwrap_or_else(|p| p.into_inner());
                    streams.chunker.drain_due(now)
                };
                flush(inner, id, entry, due);
            }
            ReadOutcome::Ended | ReadOutcome::Failed(_) => {
                // Everything still held goes out first, due or not: bytes below the size bound
                // would otherwise wait for a deadline that no longer matters, and FR-022 puts
                // them before the exit.
                let rest = {
                    let mut streams = entry.streams.lock().unwrap_or_else(|p| p.into_inner());
                    streams.chunker.drain_all()
                };
                flush(inner, id, entry, rest);

                let status = match entry.control.reap() {
                    Some(Exit::Code(code)) => ExitStatus::Exited { code },
                    Some(Exit::Signal(signal)) => ExitStatus::Signalled { signal },
                    // The descriptor closed and the process has not been reaped yet. Treated as
                    // an ordinary end: a task whose output is over has ended as far as a client
                    // is concerned, and waiting here would hold the thread open indefinitely.
                    None => ExitStatus::Exited { code: 0 },
                };
                // Cancel first. A task that ended on its own must not be signalled by a
                // deadline its own stop registered, and the identity may be reused.
                inner.escalations.cancel(id);
                emit_exit(inner, id, status);
                // **After** the exit is on the wire, never before.
                //
                // The identity is what `execution/terminate` and `execution/attach` reach a task
                // by, so releasing it earlier would make a client that terminates and then
                // attaches -- to collect the last of the output -- find nothing, and would turn
                // FR-019's already-exited terminate into `-32006` during the very window FR-019
                // is about.
                //
                // Releasing it at all is what makes FR-023 and SC-014 hold: a hundred
                // start-and-exit cycles must leave the same number of live identities as they
                // started with, and a record that only ever grows is the leak that criterion
                // exists to catch.
                release(inner, id);
                return;
            }
        }
    }
}

impl Drop for TaskService {
    fn drop(&mut self) {
        self.close();
    }
}
