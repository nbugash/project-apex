//! The OpenSSH-backed transport.
//!
//! Its internals are private modules: nothing outside this directory may depend on the
//! codec, the registry or the send queue. The application layer sees `RequestTransport` and
//! nothing else, which is what makes the mock a drop-in rather than a parallel
//! implementation.
//!
//! The idea the rest follows from: **the connection is a resource with a lifetime, and
//! everything keyed to it dies with it.** When the child exits, the reader ends, the writer
//! ends, and the registry resolves every outstanding request as `ConnectionLost` before a
//! new attempt starts. Nothing survives a reconnect, so there is no partially valid state to
//! reconcile.

mod classify;
mod framing;
mod registry;
mod sendq;
pub mod spawner;

pub use classify::classify;
pub use spawner::{parse_version, OpenSshSpawner, ASKPASS_MIN_VERSION};

use crate::application::ports::spawner::{ProcessSpawner, SpawnError, SpawnSpec};
use crate::application::ports::transport::{Request, RequestTransport};
use crate::domain::connection::ConnectionState;
use crate::domain::failure::MAX_STDERR_BYTES;
use crate::domain::request::{
    Priority, RequestId, RequestOutcome, DEFAULT_TIMEOUT_SECS, ERR_INTERNAL, ERR_PAYLOAD_TOO_LARGE,
};
use bytes::BytesMut;
use framing::{FrameCodec, FrameError};
use registry::Registry;
use sendq::SendQueue;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

/// One live connection's moving parts.
struct Live {
    queue: Arc<SendQueue>,
    /// Joined on teardown so no thread outlives the transport (SC-003).
    threads: Vec<std::thread::JoinHandle<()>>,
}

pub struct SshTransport {
    registry: Arc<Registry>,
    live: Mutex<Option<Live>>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    spawner: Arc<dyn ProcessSpawner>,
    spec: SpawnSpec,
    /// Collected for classification when the child ends.
    stderr: Arc<Mutex<String>>,
}

impl SshTransport {
    pub fn new(spawner: Arc<dyn ProcessSpawner>, spec: SpawnSpec) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Unknown);
        Self {
            registry: Arc::new(Registry::new()),
            live: Mutex::new(None),
            state_tx,
            state_rx,
            spawner,
            spec,
            stderr: Arc::new(Mutex::new(String::new())),
        }
    }

    /// FR-005. Refused at startup rather than discovered at the first connection failure.
    pub fn preflight(&self) -> Result<String, SpawnError> {
        self.spawner.preflight()
    }

    /// Every transition is published, so a subscriber's view cannot diverge from ours.
    pub fn observe(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    pub fn outstanding(&self) -> usize {
        self.registry.len()
    }

    /// Establish the connection and start the reader and writer.
    ///
    /// They are separate threads because they have independent failure modes and independent
    /// backpressure: a blocked writer — the child not draining stdin — must not stop replies
    /// being read, or a full pipe deadlocks both directions at once.
    pub fn connect(&self) -> Result<(), SpawnError> {
        let _ = self.state_tx.send(ConnectionState::Connecting);

        let child = match self.spawner.spawn(&self.spec) {
            Ok(c) => c,
            Err(e) => {
                let _ = self.state_tx.send(ConnectionState::Disconnected);
                return Err(e);
            }
        };

        let queue = Arc::new(SendQueue::new());
        let mut threads = Vec::new();

        // Writer: one thread owns stdin. Two writers would interleave bytes and corrupt
        // both frames, and the corruption would surface as a length mismatch far from its
        // cause.
        {
            let queue = queue.clone();
            let mut stdin = child.stdin;
            threads.push(std::thread::spawn(move || {
                while let Some(frame) = queue.pop_blocking() {
                    if stdin.write_all(&frame).is_err() || stdin.flush().is_err() {
                        break; // the child went away; the reader will notice too
                    }
                }
            }));
        }

        // Reader: decodes untrusted bytes and resolves requests.
        {
            let registry = self.registry.clone();
            let state_tx = self.state_tx.clone();
            let queue_for_eof = queue.clone();
            let mut stdout = child.stdout;
            threads.push(std::thread::spawn(move || {
                let mut codec = FrameCodec::new();
                let mut buf = BytesMut::new();
                let mut chunk = [0u8; 8192];
                loop {
                    match stdout.read(&mut chunk) {
                        Ok(0) | Err(_) => break, // EOF: the child ended
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                    loop {
                        match codec.decode(&mut buf) {
                            Ok(Some(frame)) => deliver(&registry, &frame.0),
                            Ok(None) => break,
                            // A refused frame is refused alone. The codec has already left
                            // the buffer at a boundary, so the next frame still reads.
                            Err(FrameError::TooLarge(n)) => {
                                crate::logging::warn(&format!(
                                    "refused a frame declaring {n} bytes"
                                ));
                            }
                            Err(FrameError::Malformed(why)) => {
                                crate::logging::warn(&format!("discarded a frame: {why}"));
                            }
                        }
                    }
                }

                // The single observation: the child ended. Everything keyed to the
                // connection dies with it.
                queue_for_eof.close();
                registry.fail_all(RequestOutcome::ConnectionLost);
                let _ = state_tx.send(ConnectionState::Disconnected);
            }));
        }

        // Stderr is drained so a chatty child cannot block on a full pipe, and bounded so it
        // cannot exhaust memory.
        {
            let sink = self.stderr.clone();
            let mut stderr = child.stderr;
            threads.push(std::thread::spawn(move || {
                let mut text = String::new();
                let _ = stderr.read_to_string(&mut text);
                let mut guard = sink.lock().expect("stderr lock");
                guard.push_str(&text);
                if guard.len() > MAX_STDERR_BYTES {
                    let cut = guard.len() - MAX_STDERR_BYTES;
                    *guard = guard[cut..].to_string();
                }
            }));
        }

        *self.live.lock().expect("live lock") = Some(Live { queue, threads });
        let _ = self.state_tx.send(ConnectionState::Connected);
        Ok(())
    }

    /// Whatever the child wrote to stderr, bounded.
    pub fn stderr_tail(&self) -> String {
        self.stderr.lock().expect("stderr lock").clone()
    }

    /// Publish that the supervisor is waiting before another attempt.
    pub fn report_retrying(&self, attempt: u32, next_in_secs: u64) {
        let _ = self.state_tx.send(ConnectionState::Retrying {
            attempt,
            next_in_secs,
        });
    }

    /// Tear the connection down.
    ///
    /// `ControlPersist=1h` outlives this process by design, so without an explicit teardown
    /// the application orphans a master holding the connection open (§3.1).
    pub fn shutdown(&self) {
        let live = self.live.lock().expect("live lock").take();
        if let Some(live) = live {
            live.queue.close();
            self.registry.fail_all(RequestOutcome::ConnectionLost);
            for t in live.threads {
                let _ = t.join();
            }
        }
        // Best effort: the master may already be gone, and its absence is not an error.
        let _ = std::process::Command::new("ssh")
            .arg("-O")
            .arg("exit")
            .arg(format!("{}@{}", self.spec.user, self.spec.host))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = self.state_tx.send(ConnectionState::Disconnected);
    }
}

impl Drop for SshTransport {
    fn drop(&mut self) {
        // Nothing outlives the transport, even on an unclean path (SC-003).
        if let Ok(mut guard) = self.live.lock() {
            if let Some(live) = guard.take() {
                live.queue.close();
                for t in live.threads {
                    let _ = t.join();
                }
            }
        }
    }
}

/// Match one reply to its request.
fn deliver(registry: &Registry, body: &str) {
    let Some(id) = reply_id(body) else {
        return; // a notification, not a reply: no id, nothing to correlate
    };
    let outcome = if let Some(err) = extract_object(body, "\"error\"") {
        RequestOutcome::Failed {
            code: extract_number(&err, "\"code\"").unwrap_or(ERR_INTERNAL),
            message: extract_string(&err, "\"message\"").unwrap_or_default(),
        }
    } else {
        RequestOutcome::Answered(body.to_string())
    };
    // A reply matching nothing is discarded without disturbing anything in flight: normal
    // for a request that already timed out or was withdrawn.
    registry.resolve(&RequestId(id), outcome);
}

#[allow(async_fn_in_trait)]
impl RequestTransport for SshTransport {
    async fn send(&self, request: Request) -> RequestOutcome {
        // Refused before transmission. An oversized frame is never partially written, and
        // the caller learns the cap rather than watching everything behind it stall.
        let codec = FrameCodec::new();

        // Registration precedes transmission (FR-011), so a reply arriving the instant the
        // write completes is still matched.
        let (id, awaiting) = self.registry.register();
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":"{id}","method":"{}","params":{}}}"#,
            request.method, request.params
        );
        let frame = match codec.encode(&body) {
            Ok(f) => f,
            Err(FrameError::TooLarge(n)) => {
                self.registry.resolve(&id, RequestOutcome::TimedOut); // clear the entry
                return RequestOutcome::Failed {
                    code: ERR_PAYLOAD_TOO_LARGE,
                    message: format!("payload of {n} bytes exceeds the frame limit"),
                };
            }
            Err(FrameError::Malformed(why)) => {
                self.registry.resolve(&id, RequestOutcome::TimedOut);
                return RequestOutcome::Failed {
                    code: ERR_INTERNAL,
                    message: why,
                };
            }
        };

        {
            let guard = self.live.lock().expect("live lock");
            let Some(live) = guard.as_ref() else {
                // Never queue for a connection that may never return.
                self.registry.resolve(&id, RequestOutcome::ConnectionLost);
                return RequestOutcome::ConnectionLost;
            };
            live.queue.push(request.priority, frame);
        }

        let limit = request
            .timeout
            .unwrap_or(Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        match tokio::time::timeout(limit, awaiting).await {
            Ok(Ok(outcome)) => outcome,
            // The sender was dropped without sending: the connection went away.
            Ok(Err(_)) => RequestOutcome::ConnectionLost,
            Err(_) => {
                // Remove the entry: a timed-out request must stop occupying the registry
                // whether or not a late reply ever arrives.
                self.registry.resolve(&id, RequestOutcome::TimedOut);
                RequestOutcome::TimedOut
            }
        }
    }

    fn withdraw(&self, id: &RequestId) {
        // §4.5: tell the remote side so it can stop work. Best effort by design — the
        // caller is released regardless.
        if let Ok(guard) = self.live.lock() {
            if let Some(live) = guard.as_ref() {
                let body = format!(
                    r#"{{"jsonrpc":"2.0","method":"$/cancelRequest","params":{{"id":"{id}"}}}}"#
                );
                if let Ok(frame) = FrameCodec::new().encode(&body) {
                    live.queue.push(Priority::Interactive, frame);
                }
            }
        }
        // Resolving an unknown or already-resolved id is a no-op, which is what makes
        // losing the race against a reply harmless.
        self.registry.resolve(id, RequestOutcome::Withdrawn);
    }

    fn state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }
}

// --- Minimal JSON probing.
//
// Deliberately not a full parse into a typed structure: the transport must correlate a reply
// whose body it does not understand, because understanding bodies is the engine's business.
// It reads the id and the error, and passes everything else through untouched.

fn reply_id(body: &str) -> Option<String> {
    extract_string(body, "\"id\"")
}

fn extract_string(haystack: &str, key: &str) -> Option<String> {
    let at = haystack.find(key)? + key.len();
    let rest = haystack[at..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_number(haystack: &str, key: &str) -> Option<i32> {
    let at = haystack.find(key)? + key.len();
    let rest = haystack[at..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_object(haystack: &str, key: &str) -> Option<String> {
    let at = haystack.find(key)? + key.len();
    let rest = haystack[at..].trim_start().strip_prefix(':')?.trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_carries_its_id() {
        assert_eq!(
            reply_id(r#"{"jsonrpc":"2.0","id":"req_7","result":{}}"#),
            Some("req_7".into())
        );
    }

    #[test]
    fn a_notification_has_no_id_to_correlate() {
        assert_eq!(
            reply_id(r#"{"jsonrpc":"2.0","method":"log/onMessage"}"#),
            None
        );
    }

    #[test]
    fn an_error_reply_yields_its_code_and_message() {
        let body =
            r#"{"jsonrpc":"2.0","id":"req_1","error":{"code":-32003,"message":"not found"}}"#;
        let err = extract_object(body, "\"error\"").expect("error object");
        assert_eq!(extract_number(&err, "\"code\""), Some(-32003));
        assert_eq!(
            extract_string(&err, "\"message\""),
            Some("not found".into())
        );
    }

    #[test]
    fn a_result_containing_the_word_error_is_not_an_error() {
        // The probe must not be fooled by a payload that merely mentions it.
        let body = r#"{"jsonrpc":"2.0","id":"req_1","result":{"text":"error handling"}}"#;
        assert!(extract_object(body, "\"error\"").is_none());
    }

    #[test]
    fn a_nested_error_object_is_extracted_whole() {
        let body = r#"{"id":"r","error":{"code":-1,"data":{"nested":{"deep":1}},"message":"m"}}"#;
        let err = extract_object(body, "\"error\"").expect("error object");
        assert!(err.ends_with('}'));
        assert_eq!(extract_string(&err, "\"message\""), Some("m".into()));
    }
}
