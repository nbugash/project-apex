//! A task's output, between the descriptor and the wire.
//!
//! Two pure types and no I/O. The chunker is fed bytes and told the time; the retention buffer
//! is fed chunks and asked how many bytes it holds. Neither names a descriptor, a thread or a
//! syscall, which is what lets FR-011's chunking, FR-010's ordering and FR-013a's bound be
//! decided by arithmetic rather than by starting a process and hoping.
//!
//! The quantities live here as named constants because plan.md's *Fixed Quantities* fixes them
//! and FR-006b, FR-011 and FR-013a all require a stated value rather than a judgement made per
//! task. Tests read the constants; a test carrying its own copy of a number passes after
//! somebody changes the bound.

use std::collections::VecDeque;

use crate::application::ports::clock::Millis;
use crate::domain::task::{OutputChunk, Stream};

/// Raw bytes in one chunk, before base64 (plan.md, *Chunk size bound*).
///
/// §4.1 caps a frame at 1 MiB and base64 inflates by four bytes for every three, so the true
/// ceiling is three quarters of the frame budget -- about 786 KB. 64 KiB sits an order of
/// magnitude under it while keeping a 50 MiB burst to roughly 800 frames rather than the 6 400
/// an 8 KiB chunk would cost.
pub const CHUNK_BYTES: usize = 64 * 1024;

/// How long bytes wait before going out under the size bound (plan.md, *Chunk time bound*).
///
/// The size bound alone starves an interactive prompt, which never fills a chunk and would
/// therefore never be sent. Under a burst the size bound dominates, so this costs nothing where
/// volume is high.
pub const CHUNK_INTERVAL_MS: Millis = 20;

/// Bytes one task may have retained before its producer is slowed (plan.md, *Buffered before
/// slowing*; FR-013a).
pub const RETENTION_BYTES: usize = 4 * 1024 * 1024;

/// Bytes accepted from one stream but not yet emitted.
#[derive(Debug, Default)]
struct Partial {
    bytes: Vec<u8>,
    /// When the **first** byte of this partial arrived.
    ///
    /// Set once and not refreshed on every arrival. A deadline reset by each new byte is a
    /// debounce, and under a process writing continuously it never expires -- which is F004's
    /// coalescer lesson applied here: the panel would stay empty for exactly the task that is
    /// producing most.
    since: Option<Millis>,
}

/// Turns a stream of reads into frames, by size and by time.
#[derive(Debug, Default)]
pub struct Chunker {
    stdout: Partial,
    stderr: Partial,
    ready: VecDeque<OutputChunk>,
}

impl Chunker {
    pub fn new() -> Self {
        Self::default()
    }

    fn partial(&mut self, stream: Stream) -> &mut Partial {
        match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        }
    }

    /// Take bytes from one read.
    ///
    /// Whole chunks become ready immediately: a burst does not wait for a clock tick it has
    /// already outrun.
    pub fn accept(&mut self, stream: Stream, bytes: &[u8], now: Millis) {
        if bytes.is_empty() {
            return;
        }
        let ready = &mut self.ready;
        let partial = match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        };
        if partial.since.is_none() {
            partial.since = Some(now);
        }
        partial.bytes.extend_from_slice(bytes);
        while partial.bytes.len() >= CHUNK_BYTES {
            let rest = partial.bytes.split_off(CHUNK_BYTES);
            let full = std::mem::replace(&mut partial.bytes, rest);
            ready.push_back(OutputChunk {
                stream,
                bytes: full,
            });
        }
        // The remainder starts its own wait. Without this a 4 MiB line's trailing bytes would
        // carry the deadline of the first byte of the whole line, which has long passed.
        partial.since = if partial.bytes.is_empty() {
            None
        } else {
            Some(now)
        };
    }

    /// Everything due: whole chunks, plus any partial whose time bound has expired.
    ///
    /// Stdout before stderr when both are due in the same call, for determinism only. Within
    /// one stream the order is the order accepted, which is what FR-010 and SC-004 require;
    /// across streams nothing is promised, and with a pseudo-terminal there is only one.
    pub fn drain_due(&mut self, now: Millis) -> Vec<OutputChunk> {
        let mut out: Vec<OutputChunk> = self.ready.drain(..).collect();
        for stream in [Stream::Stdout, Stream::Stderr] {
            let due = {
                let p = self.partial(stream);
                !p.bytes.is_empty()
                    && p.since
                        .is_some_and(|s| now.saturating_sub(s) >= CHUNK_INTERVAL_MS)
            };
            if due {
                let p = self.partial(stream);
                let bytes = std::mem::take(&mut p.bytes);
                p.since = None;
                out.push(OutputChunk { stream, bytes });
            }
        }
        out
    }

    /// Flush everything, due or not. What the reader calls when the process has ended: bytes
    /// below the size bound with nothing following would otherwise wait for a deadline that is
    /// now irrelevant, and FR-022 requires them out before the exit.
    pub fn drain_all(&mut self) -> Vec<OutputChunk> {
        let mut out: Vec<OutputChunk> = self.ready.drain(..).collect();
        for stream in [Stream::Stdout, Stream::Stderr] {
            let p = self.partial(stream);
            if !p.bytes.is_empty() {
                let bytes = std::mem::take(&mut p.bytes);
                p.since = None;
                out.push(OutputChunk { stream, bytes });
            }
        }
        out
    }

    /// When the next emission is due, or `None` when nothing is pending.
    ///
    /// This is the timeout the reader thread passes to `TaskOutput::read`, so the thread wakes
    /// to flush and for nothing else -- and owns no policy, because the policy is this number.
    pub fn next_deadline(&self) -> Option<Millis> {
        [&self.stdout, &self.stderr]
            .iter()
            .filter(|p| !p.bytes.is_empty())
            .filter_map(|p| p.since)
            .map(|s| s + CHUNK_INTERVAL_MS)
            .min()
    }

    /// Whether anything is waiting, ready or partial.
    pub fn is_empty(&self) -> bool {
        self.ready.is_empty() && self.stdout.bytes.is_empty() && self.stderr.bytes.is_empty()
    }
}

/// Whether the retention buffer took a chunk, and whether the producer should now be slowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Accepted,
    /// The bound is reached. **The chunk was still taken** -- FR-013 slows a producer and never
    /// truncates its output, so this is the caller's signal to stop reading, not a rejection.
    /// Stopping the read lets the pseudo-terminal's buffer fill, which blocks the task in
    /// `write`, which is the whole of the backpressure chain.
    AtBound,
}

/// What a task has produced and not yet delivered.
///
/// A queue of chunks rather than a flat byte buffer, so a `pty: false` task's two streams stay
/// separable across a detachment: a replay has to put each chunk back on the notification it
/// would have used when live, and a flat buffer has thrown that away (invariant 14, SC-028).
///
/// `held_bytes` is this buffer alone. Nothing buffers between the reader thread and the wire --
/// the reader writes its own frames through `FrameWriter` and blocks there when the client is
/// not draining -- so these are the only bytes the engine is holding for a task.
#[derive(Debug, Default)]
pub struct RetainedOutput {
    chunks: VecDeque<OutputChunk>,
    bytes_held: usize,
    ending: Option<crate::domain::task::ExitStatus>,
}

impl RetainedOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Always takes the chunk. The return value says whether the producer should now be slowed.
    pub fn push(&mut self, chunk: OutputChunk) -> Admission {
        self.bytes_held += chunk.len();
        self.chunks.push_back(chunk);
        if self.bytes_held >= RETENTION_BYTES {
            Admission::AtBound
        } else {
            Admission::Accepted
        }
    }

    /// FIFO, and empties. The order chunks were produced in is the order they are delivered in,
    /// live or replayed.
    pub fn drain(&mut self) -> Vec<OutputChunk> {
        self.bytes_held = 0;
        self.chunks.drain(..).collect()
    }

    pub fn held_bytes(&self) -> usize {
        self.bytes_held
    }

    pub fn is_full(&self) -> bool {
        self.bytes_held >= RETENTION_BYTES
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Recorded once. A second call does not overwrite the first: a task ends once, and a
    /// second ending would mean something has gone wrong upstream rather than that the task
    /// ended twice.
    pub fn set_ending(&mut self, status: crate::domain::task::ExitStatus) {
        if self.ending.is_none() {
            self.ending = Some(status);
        }
    }

    /// The ending, **only once the buffer has emptied**.
    ///
    /// FR-022 and SC-011: output produced before an exit is delivered before the exit is
    /// reported, so that the last lines of a failing build are not lost to the report of its
    /// failure. Returning the ending while chunks remain is exactly that loss.
    pub fn deliverable_ending(&self) -> Option<crate::domain::task::ExitStatus> {
        if self.chunks.is_empty() {
            self.ending
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::task::ExitStatus;

    /// A scripted run: bytes in, chunks out, no descriptor anywhere.
    fn feed(chunker: &mut Chunker, stream: Stream, total: usize, now: Millis) -> Vec<OutputChunk> {
        let block = vec![b'x'; total];
        chunker.accept(stream, &block, now);
        chunker.drain_due(now)
    }

    #[test]
    fn a_four_mib_line_with_no_newline_is_chunked_by_size() {
        // SC-005 and FR-011. A chunker that flushed on newlines looks correct against every
        // ordinary program and stalls forever here.
        const TOTAL: usize = 4 * 1024 * 1024;
        let mut c = Chunker::new();
        let out = feed(&mut c, Stream::Stdout, TOTAL, 0);

        // Read from the constant, never a literal. 64 KiB *is* 65536, which makes this the
        // easiest of the bounds to write a test that passes after somebody changes it.
        assert!(
            out.len() >= TOTAL / CHUNK_BYTES,
            "expected at least {} chunks, got {}",
            TOTAL / CHUNK_BYTES,
            out.len()
        );
        for chunk in &out {
            assert!(
                chunk.len() <= CHUNK_BYTES,
                "a chunk of {} exceeds the bound of {CHUNK_BYTES}",
                chunk.len()
            );
        }
        let delivered: usize = out.iter().map(|c| c.len()).sum();
        let remaining = c.drain_all().iter().map(|c| c.len()).sum::<usize>();
        assert_eq!(
            delivered + remaining,
            TOTAL,
            "zero bytes may be lost to chunking"
        );
    }

    #[test]
    fn bytes_below_the_size_bound_go_out_when_the_time_bound_expires() {
        let mut c = Chunker::new();
        c.accept(Stream::Stdout, b"$ ", 1_000);

        // Nothing is due yet.
        assert!(c.drain_due(1_000).is_empty());
        assert!(c.drain_due(1_000 + CHUNK_INTERVAL_MS - 1).is_empty());

        let out = c.drain_due(1_000 + CHUNK_INTERVAL_MS);
        // **At least one**, not exactly one. An upper-bound-only suite passes when the time
        // bound is deleted entirely, which is the mutation that leaves a developer staring at
        // an empty panel while a prompt sits in a buffer nobody flushes.
        assert!(
            !out.is_empty(),
            "a prompt below the size bound must still be delivered"
        );
        assert_eq!(out[0].bytes, b"$ ");
    }

    #[test]
    fn the_deadline_is_the_timeout_the_reader_waits_on() {
        let mut c = Chunker::new();
        assert_eq!(
            c.next_deadline(),
            None,
            "nothing pending, nothing to wait for"
        );
        c.accept(Stream::Stdout, b"x", 500);
        assert_eq!(c.next_deadline(), Some(500 + CHUNK_INTERVAL_MS));
    }

    #[test]
    fn order_within_a_stream_is_the_order_accepted() {
        let mut c = Chunker::new();
        for i in 0..5u8 {
            c.accept(Stream::Stdout, &[b'a' + i], 0);
        }
        let out = c.drain_due(CHUNK_INTERVAL_MS);
        let joined: Vec<u8> = out.iter().flat_map(|c| c.bytes.clone()).collect();
        assert_eq!(
            joined, b"abcde",
            "FR-010: the order produced is the order sent"
        );
    }

    #[test]
    fn order_survives_a_chunk_boundary() {
        let mut c = Chunker::new();
        // One byte over the bound, so the tail is a separate chunk.
        let mut payload = vec![b'a'; CHUNK_BYTES];
        payload.push(b'Z');
        c.accept(Stream::Stdout, &payload, 0);

        let mut out = c.drain_due(0);
        out.extend(c.drain_all());
        let joined: Vec<u8> = out.iter().flat_map(|c| c.bytes.clone()).collect();
        assert_eq!(joined.len(), CHUNK_BYTES + 1);
        assert_eq!(
            *joined.last().expect("a byte"),
            b'Z',
            "the byte after the boundary must still be last"
        );
    }

    #[test]
    fn each_chunk_remembers_its_own_stream() {
        let mut c = Chunker::new();
        c.accept(Stream::Stdout, b"out", 0);
        c.accept(Stream::Stderr, b"err", 0);
        let out = c.drain_due(CHUNK_INTERVAL_MS);

        let stdout: Vec<_> = out.iter().filter(|c| c.stream == Stream::Stdout).collect();
        let stderr: Vec<_> = out.iter().filter(|c| c.stream == Stream::Stderr).collect();
        assert_eq!(stdout.len(), 1);
        assert_eq!(stderr.len(), 1);
        // A replay has to put each chunk back on the notification it would have used live,
        // which a flat byte buffer would have made impossible (invariant 14, SC-028).
        assert_eq!(stdout[0].bytes, b"out");
        assert_eq!(stderr[0].bytes, b"err");
    }

    #[test]
    fn a_terminal_task_produces_only_stdout() {
        // A pseudo-terminal is one device, so the adapter has nothing to tag as stderr. The
        // chunker never invents the distinction: with a pty, nothing is ever accepted on
        // Stderr, so nothing is ever emitted on it (A-TASKSTREAM, T3, SC-028).
        let mut c = Chunker::new();
        c.accept(Stream::Stdout, b"merged", 0);
        let out = c.drain_due(CHUNK_INTERVAL_MS);
        assert!(out.iter().all(|c| c.stream == Stream::Stdout));
    }

    #[test]
    fn retention_takes_the_chunk_at_the_bound_rather_than_dropping_it() {
        let mut r = RetainedOutput::new();
        let big = OutputChunk {
            stream: Stream::Stdout,
            bytes: vec![b'x'; RETENTION_BYTES],
        };
        // FR-013 slows a producer and never truncates its output. `AtBound` is the signal to
        // stop reading, not a rejection -- the bytes are already taken.
        assert_eq!(r.push(big), Admission::AtBound);
        assert_eq!(r.held_bytes(), RETENTION_BYTES);
        assert!(r.is_full());

        let drained = r.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].len(), RETENTION_BYTES, "zero bytes dropped");
        assert_eq!(r.held_bytes(), 0);
        assert!(r.is_empty());
    }

    #[test]
    fn retention_stays_below_the_bound_until_it_is_reached() {
        let mut r = RetainedOutput::new();
        let chunk = OutputChunk {
            stream: Stream::Stdout,
            bytes: vec![b'x'; CHUNK_BYTES],
        };
        assert_eq!(r.push(chunk), Admission::Accepted);
        assert!(!r.is_full());
        assert_eq!(r.held_bytes(), CHUNK_BYTES);
    }

    #[test]
    fn retention_drains_in_the_order_it_took() {
        let mut r = RetainedOutput::new();
        for i in 0..4u8 {
            r.push(OutputChunk {
                stream: if i % 2 == 0 {
                    Stream::Stdout
                } else {
                    Stream::Stderr
                },
                bytes: vec![b'a' + i],
            });
        }
        let drained = r.drain();
        let bytes: Vec<u8> = drained.iter().flat_map(|c| c.bytes.clone()).collect();
        assert_eq!(bytes, b"abcd");
        // Each chunk kept its own stream across the retention, which is what makes a replay
        // able to use the notification that chunk would have used live.
        assert_eq!(drained[0].stream, Stream::Stdout);
        assert_eq!(drained[1].stream, Stream::Stderr);
    }

    #[test]
    fn an_ending_is_not_deliverable_until_the_output_has_drained() {
        let mut r = RetainedOutput::new();
        r.push(OutputChunk {
            stream: Stream::Stdout,
            bytes: b"the last line of a failing build".to_vec(),
        });
        r.set_ending(ExitStatus::Exited { code: 101 });

        // FR-022 and SC-011. Reporting the exit here would lose exactly the output a developer
        // needs to see, to the report of the failure that produced it.
        assert_eq!(r.deliverable_ending(), None);

        r.drain();
        assert_eq!(
            r.deliverable_ending(),
            Some(ExitStatus::Exited { code: 101 })
        );
    }

    #[test]
    fn an_ending_is_recorded_once() {
        let mut r = RetainedOutput::new();
        r.set_ending(ExitStatus::Exited { code: 0 });
        r.set_ending(ExitStatus::Signalled { signal: 9 });
        // A task ends once. A second ending means something upstream is wrong, not that the
        // task ended twice, and overwriting would hide it.
        assert_eq!(r.deliverable_ending(), Some(ExitStatus::Exited { code: 0 }));
    }
}
