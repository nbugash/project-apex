//! Reading back what actually reached the wire.
//!
//! Every claim F010 makes about delivery -- that bytes survive, that order holds, that an exit
//! comes last -- is a claim about frames on the client's side of the boundary, not about what some
//! internal buffer held. So the assertions are made against a sink the writer wrote to, parsed
//! back through the same `Content-Length` framing a client would parse.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

/// Collects whatever is written, so assertions are about frames that reached the wire.
#[derive(Clone, Default)]
pub struct Sink(pub Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("sink").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One frame: its method, and its `data` decoded back to bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub method: String,
    pub data: Option<Vec<u8>>,
    pub exit_code: Option<i64>,
    pub signal: Option<String>,
}

pub fn frames_of(sink: &Sink) -> Vec<Frame> {
    let raw = sink.0.lock().expect("sink").clone();
    // Lossy is safe here and only here: a frame's *body* is JSON, whose every byte is ASCII
    // because output travels base64. The bytes this is used to check are inside that base64, and
    // they are decoded below rather than read out of this string.
    let text = String::from_utf8_lossy(&raw).into_owned();
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("\r\n\r\n") {
        let header = &rest[..at];
        let len: usize = header
            .rsplit(' ')
            .next()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0);
        let body_start = at + 4;
        if body_start + len > rest.len() {
            break;
        }
        let body = &rest[body_start..body_start + len];
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
            let method = v["method"].as_str().unwrap_or("").to_string();
            let data = v["params"]["data"]
                .as_str()
                .map(|d| apex_protocol::base64::decode(d).expect("base64"));
            out.push(Frame {
                method,
                data,
                exit_code: v["params"]["exit_code"].as_i64(),
                signal: v["params"]["signal"].as_str().map(str::to_string),
            });
        }
        rest = &rest[body_start + len..];
    }
    out
}

/// Every output byte delivered, in frame order.
///
/// Both streams, because for a pipe task they are two methods and the order that matters is the
/// order they reached the wire. A terminal task produces only `onStdout` by construction
/// (A-TASKSTREAM), so including `onStderr` costs nothing there and is not a special case.
pub fn delivered_bytes(frames: &[Frame]) -> Vec<u8> {
    frames
        .iter()
        .filter(|f| f.method == "execution/onStdout" || f.method == "execution/onStderr")
        .filter_map(|f| f.data.as_ref())
        .flat_map(|d| d.iter().copied())
        .collect()
}
