//! The one thing that writes to the client.
//!
//! Until F004 the engine had a single writer -- the stdio loop -- so nothing needed to
//! coordinate. The watcher thread is the second, and §4.6 makes this one pipe and one queue:
//! a frame interleaved with a reply is a corrupt stream rather than a slow one.
//!
//! The lock is held for exactly one frame and released. That is the whole design, and it is
//! also the thing FR-016 measures -- event delivery must not delay interactive traffic, which
//! is a claim about how long this lock is held rather than about how fast the watcher is.
//!
//! # The fairness gate (F010)
//!
//! A mutex is first-come by acquisition. That was sufficient while every producer was small and
//! infrequent, and stopped being sufficient the moment one was neither: a task emitting tens of
//! megabytes takes this lock hundreds of times, and a completion response arriving behind those
//! acquisitions waits for all of them.
//!
//! §4.6 requires interactive traffic to win the race to the wire, and states that the engine
//! does **not** queue. The reason is that a producer blocked on this lock is a producer that has
//! stopped producing: the reader thread stops reading its pseudo-terminal, the terminal's buffer
//! fills, and the task blocks in `write`. That chain is the whole of FR-013, and it exists only
//! because nothing buffers between the producer and the wire. A queue would end it -- and would
//! also split a task's output from its exit into two priority classes, letting the exit overtake
//! the output FR-022 requires it to follow.
//!
//! So priority is granted by making a bulk producer **wait its turn** rather than by holding its
//! output. Nothing here buffers a byte.

use std::io::{self, Write};
use std::sync::{Condvar, Mutex};

/// How many times a bulk writer yields before writing anyway (plan.md, *Fixed Quantities*).
///
/// Priority is a strong preference and never a monopoly. Typing cannot starve a build -- a key
/// held at its repeat rate raises the count a few dozen times a second for microseconds each --
/// but F007's language servers can: §4.6 names LSP responses interactive, and a server streaming
/// diagnostics across a large workspace is a *sustained* interactive producer. Without this
/// bound a build's output would stall for as long as indexing lasts.
pub const CONSECUTIVE_YIELDS: u32 = 8;

pub struct FrameWriter {
    sink: Mutex<Box<dyn Write + Send>>,
    /// How many writers with interactive traffic are waiting for, or holding, the sink.
    ///
    /// Its own lock, deliberately. A bulk writer has to be able to observe this while the sink
    /// mutex is held by the interactive writer it is yielding to; one lock covering both would
    /// mean waiting for the very thing being waited on.
    waiting: Mutex<usize>,
    /// Notified when `waiting` reaches zero.
    clear: Condvar,
}

impl FrameWriter {
    pub fn new(sink: Box<dyn Write + Send>) -> Self {
        Self {
            sink: Mutex::new(sink),
            waiting: Mutex::new(0),
            clear: Condvar::new(),
        }
    }

    pub fn to_stdout() -> Self {
        Self::new(Box::new(io::stdout()))
    }

    /// Write one complete frame, then flush, then release.
    ///
    /// A poisoned lock means a writer panicked mid-frame. The stream is already suspect, so
    /// carrying on is the honest choice: the alternative is a panic here that hides the
    /// original one.
    fn write_frame(&self, frame: &[u8]) -> io::Result<()> {
        let mut sink = match self.sink.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        sink.write_all(frame)?;
        sink.flush()
    }

    fn lock_waiting(&self) -> std::sync::MutexGuard<'_, usize> {
        match self.waiting.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Write a frame that must not wait: a command reply, a file event, an LSP response.
    ///
    /// Named rather than left as a plain `write`, so that **every call site chooses**. A
    /// default would be taken by whoever adds the next producer without thinking about which
    /// class it belongs to -- which is how the design this replaced came to classify a task's
    /// exit as interactive and let it overtake the task's own output.
    ///
    /// Takes `&self` so it can be shared behind an `Arc` without any caller needing mutable
    /// access -- which is what lets the stdio loop and the watcher thread both hold one without
    /// either owning it.
    pub fn write_interactive(&self, frame: &[u8]) -> io::Result<()> {
        // Raised *before* the sink is taken, so a bulk writer that checks between these two
        // steps still sees a writer it should yield to.
        *self.lock_waiting() += 1;
        let result = self.write_frame(frame);
        let mut waiting = self.lock_waiting();
        *waiting -= 1;
        if *waiting == 0 {
            self.clear.notify_all();
        }
        result
    }

    /// Write a frame that may wait: a task's output, and a task's exit.
    ///
    /// **A task's `onExit` is bulk, like its output.** The two travel the same path, written by
    /// the same reader thread in the order it produced them, which is what makes it impossible
    /// for the exit to overtake the output (FR-022, SC-011). Classing the exit as interactive
    /// would reintroduce exactly that overtaking.
    pub fn write_bulk(&self, frame: &[u8]) -> io::Result<()> {
        {
            let mut waiting = self.lock_waiting();
            let mut yields = 0u32;
            while *waiting > 0 && yields < CONSECUTIVE_YIELDS {
                waiting = match self.clear.wait(waiting) {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                // A spurious wake counts as a yield. That only makes the gate less strict, and
                // the bound is a safety valve rather than a budget anyone is spending.
                yields += 1;
            }
        }
        self.write_frame(frame)
    }
}
