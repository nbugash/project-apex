//! Two writers, one pipe (§4.6, FR-016).
//!
//! Until F004 the engine had a single writer and nothing had to coordinate. The watcher thread
//! is the second. A frame interleaved with a reply is a corrupt stream rather than a slow one,
//! and the difference is invisible to any test that writes from one thread.

use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

/// A sink that records what reached it, byte for byte.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<u8>>>);

impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Deliberately one byte at a time. A sink that accepts a whole frame per call would
        // make interleaving impossible for reasons that have nothing to do with the lock, and
        // the test would pass with the lock removed.
        let mut held = self.0.lock().expect("recorder");
        held.push(buf[0]);
        Ok(1)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn frame(tag: char, len: usize) -> Vec<u8> {
    let body: String = std::iter::repeat(tag).take(len).collect();
    format!("Content-Length: {len}\r\n\r\n{body}").into_bytes()
}

#[test]
fn concurrent_writers_never_interleave_a_frame() {
    let recorder = Recorder::default();
    let seen = Arc::clone(&recorder.0);
    let writer = Arc::new(FrameWriter::new(Box::new(recorder)));

    let mut handles = Vec::new();
    for (tag, rounds) in [('a', 40), ('b', 40)] {
        let w = Arc::clone(&writer);
        handles.push(std::thread::spawn(move || {
            for _ in 0..rounds {
                w.write_interactive(&frame(tag, 64)).expect("write");
            }
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }

    let text = String::from_utf8(seen.lock().expect("recorder").clone()).expect("utf-8");
    let mut rest = text.as_str();
    let mut frames = 0;
    while !rest.is_empty() {
        let (header, tail) = rest.split_once("\r\n\r\n").expect("a framed payload");
        let len: usize = header
            .trim_start_matches("Content-Length: ")
            .parse()
            .expect("a length");
        let (body, tail) = tail.split_at(len);
        // Every frame is one character repeated. A frame with two different characters in it
        // is two writers' bytes in one payload, which is the failure this test exists for.
        let first = body.chars().next().expect("a body");
        assert!(
            body.chars().all(|c| c == first),
            "a frame carried bytes from both writers: {body:?}"
        );
        frames += 1;
        rest = tail;
    }
    assert_eq!(frames, 80, "every frame arrived whole and none was lost");
}

#[test]
fn a_frame_is_written_exactly_once() {
    let recorder = Recorder::default();
    let seen = Arc::clone(&recorder.0);
    let writer = FrameWriter::new(Box::new(recorder));
    writer.write_interactive(&frame('x', 3)).expect("write");
    assert_eq!(
        String::from_utf8(seen.lock().expect("recorder").clone()).expect("utf-8"),
        "Content-Length: 3\r\n\r\nxxx"
    );
}
