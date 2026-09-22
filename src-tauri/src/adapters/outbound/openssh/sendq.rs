//! Outbound ordering: interactive traffic ahead of background work (§4.6, FR-021).
//!
//! Two classes, not five. Nothing in the system specification distinguishes more than
//! "editor traffic" from "background work", and a priority scheme finer than its
//! requirements is one nobody can apply consistently.
//!
//! Ordering applies **between** frames, never within one. A frame already being written
//! completes first, because `Content-Length` has promised exactly that many bytes follow and
//! interrupting it corrupts the stream. The 1 MiB cap is what bounds the resulting delay.

use crate::domain::request::Priority;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

#[derive(Default)]
struct Queues {
    interactive: VecDeque<Vec<u8>>,
    background: VecDeque<Vec<u8>>,
    closed: bool,
}

/// A queue the writer drains by priority.
#[derive(Default)]
pub struct SendQueue {
    queues: Mutex<Queues>,
    /// The writer blocks here rather than polling, so an idle connection costs nothing.
    ready: Condvar,
}

impl SendQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, priority: Priority, frame: Vec<u8>) {
        let mut q = self.queues.lock().expect("send queue lock");
        if q.closed {
            return;
        }
        match priority {
            Priority::Interactive => q.interactive.push_back(frame),
            Priority::Background => q.background.push_back(frame),
        }
        self.ready.notify_one();
    }

    /// Block until a frame is available, or the queue closes.
    ///
    /// Interactive first, then background; FIFO within each. Returning `None` means the
    /// connection is going away and the writer should stop.
    pub fn pop_blocking(&self) -> Option<Vec<u8>> {
        let mut q = self.queues.lock().expect("send queue lock");
        loop {
            if let Some(f) = q
                .interactive
                .pop_front()
                .or_else(|| q.background.pop_front())
            {
                return Some(f);
            }
            if q.closed {
                return None;
            }
            q = self.ready.wait(q).expect("send queue wait");
        }
    }

    /// Stop the writer and discard anything queued.
    ///
    /// Queued frames are dropped rather than written: they belong to a connection that no
    /// longer exists, and their requests have already resolved as `ConnectionLost`.
    pub fn close(&self) {
        let mut q = self.queues.lock().expect("send queue lock");
        q.closed = true;
        q.interactive.clear();
        q.background.clear();
        self.ready.notify_all();
    }

    /// Depth, for the tests that assert a closed queue discards its work. Production never
    /// consults it: a writer that checks the depth before popping has raced by the time it
    /// pops.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        let q = self.queues.lock().expect("send queue lock");
        q.interactive.len() + q.background.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    #[test]
    fn interactive_goes_ahead_of_queued_background_work() {
        let q = SendQueue::new();
        q.push(Priority::Background, f("bg1"));
        q.push(Priority::Background, f("bg2"));
        q.push(Priority::Interactive, f("ui"));

        assert_eq!(
            q.pop_blocking(),
            Some(f("ui")),
            "interactive must not queue behind background"
        );
        assert_eq!(q.pop_blocking(), Some(f("bg1")));
        assert_eq!(q.pop_blocking(), Some(f("bg2")));
    }

    #[test]
    fn order_within_a_class_is_first_in_first_out() {
        let q = SendQueue::new();
        for n in 0..5 {
            q.push(Priority::Interactive, f(&format!("{n}")));
        }
        for n in 0..5 {
            assert_eq!(q.pop_blocking(), Some(f(&format!("{n}"))));
        }
    }

    #[test]
    fn an_interactive_frame_arriving_late_still_overtakes() {
        let q = SendQueue::new();
        for n in 0..100 {
            q.push(Priority::Background, f(&format!("bg{n}")));
        }
        q.push(Priority::Interactive, f("ui"));
        assert_eq!(
            q.pop_blocking(),
            Some(f("ui")),
            "saturating the queue with background work must not delay interactive traffic"
        );
    }

    #[test]
    fn closing_releases_a_blocked_writer() {
        let q = std::sync::Arc::new(SendQueue::new());
        let writer = {
            let q = q.clone();
            std::thread::spawn(move || q.pop_blocking())
        };
        // A real blocking wait, in a test with no runtime: the point is that a thread
        // parked in `pop_blocking` is woken by `close`, which is a thread-level property.
        #[allow(clippy::disallowed_methods)]
        std::thread::sleep(std::time::Duration::from_millis(50));
        q.close();
        assert_eq!(writer.join().unwrap(), None, "close must wake the writer");
    }

    #[test]
    fn closing_discards_queued_frames() {
        let q = SendQueue::new();
        q.push(Priority::Interactive, f("doomed"));
        q.close();
        assert!(q.is_empty());
        assert_eq!(q.pop_blocking(), None);
    }

    #[test]
    fn pushing_after_close_is_ignored() {
        let q = SendQueue::new();
        q.close();
        q.push(Priority::Interactive, f("late"));
        assert!(
            q.is_empty(),
            "a closed queue must not accept work for a dead connection"
        );
    }
}
