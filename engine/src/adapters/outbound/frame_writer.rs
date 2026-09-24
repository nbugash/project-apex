//! The one thing that writes to the client.
//!
//! Until F004 the engine had a single writer -- the stdio loop -- so nothing needed to
//! coordinate. The watcher thread is the second, and §4.6 makes this one pipe and one queue:
//! a frame interleaved with a reply is a corrupt stream rather than a slow one.
//!
//! The lock is held for exactly one frame and released. That is the whole design, and it is
//! also the thing FR-016 measures -- event delivery must not delay interactive traffic, which
//! is a claim about how long this lock is held rather than about how fast the watcher is.

use std::io::{self, Write};
use std::sync::Mutex;

pub struct FrameWriter {
    sink: Mutex<Box<dyn Write + Send>>,
}

impl FrameWriter {
    pub fn new(sink: Box<dyn Write + Send>) -> Self {
        Self {
            sink: Mutex::new(sink),
        }
    }

    pub fn to_stdout() -> Self {
        Self::new(Box::new(io::stdout()))
    }

    /// Write one complete frame, then flush, then release.
    ///
    /// Takes `&self` so it can be shared behind an `Arc` without any caller needing mutable
    /// access -- which is what lets the stdio loop and the watcher thread both hold one
    /// without either owning it.
    pub fn write(&self, frame: &[u8]) -> io::Result<()> {
        // A poisoned lock means a writer panicked mid-frame. The stream is already suspect,
        // so carrying on is the honest choice: the alternative is a panic here that hides
        // the original one.
        let mut sink = match self.sink.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        sink.write_all(frame)?;
        sink.flush()
    }
}
