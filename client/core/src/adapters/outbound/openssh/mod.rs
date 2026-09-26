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
mod registry;
mod sendq;
pub mod spawner;

pub use classify::classify;
pub use spawner::{control_options, parse_version, OpenSshSpawner, ASKPASS_MIN_VERSION};

use crate::adapters::outbound::askpass::ipc::AskpassChannel;
use crate::application::ports::notification_sink::{DiscardNotifications, NotificationSink};
use crate::application::ports::connection::{ConnectionStatusSource, StateSink};
use crate::application::ports::spawner::{ProcessSpawner, SpawnError, SpawnSpec};
use crate::application::ports::transport::{Pending, Request, RequestTransport};
use crate::application::use_cases::connect::ConnectAttempt;
use crate::application::use_cases::exchange;
use crate::domain::connection::ConnectionState;
use crate::domain::failure::{FailureCondition, MAX_STDERR_BYTES};
use crate::domain::request::{
    Priority, RequestId, RequestOutcome, Secret, ERR_INTERNAL, ERR_PAYLOAD_TOO_LARGE,
};
use bytes::BytesMut;
// The codec moved to `apex-protocol` in F002 so the engine obeys the same definition. The
// application layer still cannot see it: everything above this adapter sees `RequestTransport`.
use apex_protocol::framing::{FrameCodec, FrameError};
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
    /// The child's exit code, once it has ended. `None` while it is still running.
    ///
    /// Half of what classification needs; `stderr` is the other half. They are collected by
    /// the same thread because stderr's EOF and the child's exit are the same moment.
    exit: Arc<Mutex<Option<i32>>>,
    /// The channel a passphrase reaches OpenSSH through, when one is in play.
    askpass: Mutex<Option<Arc<AskpassChannel>>>,
    /// False below OpenSSH 8.4 — see `ASKPASS_MIN_VERSION`.
    assisted_available: std::sync::atomic::AtomicBool,
    /// Where engine-initiated frames go. Read once when the reader thread starts rather than
    /// per frame: the sink is set at composition and never changes afterwards, so a lock taken
    /// on every inbound frame would be contention bought for nothing.
    notifications: Mutex<Arc<dyn NotificationSink>>,
}

/// How long a freshly spawned child must survive before the attempt counts as established.
///
/// There is no handshake to wait for: this feature defines no connect message, and the
/// first real exchange belongs to User Story 3. So the only observation available is that
/// `ssh` has not given up — a refused credential ends the child in well under this, and a
/// working connection stays open indefinitely.
///
/// The error is asymmetric, which is why a window is acceptable at all. Too short reports
/// `Connected` for a connection that dies a moment later, and the loss path already handles
/// exactly that. Too long makes every connect feel slow, and nothing recovers that.
const SETTLE: Duration = Duration::from_millis(400);

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
            exit: Arc::new(Mutex::new(None)),
            askpass: Mutex::new(None),
            assisted_available: std::sync::atomic::AtomicBool::new(false),
            notifications: Mutex::new(Arc::new(DiscardNotifications)),
        }
    }

    /// Give the transport somewhere to put a passphrase, and say whether the assisted phase
    /// is usable at all. Both come from the composition root, which is the only place that
    /// knows the local OpenSSH version and where the helper was installed.
    /// Where to put frames the engine sent unasked.
    ///
    /// Set from the composition root, like the askpass channel and for the same reason: an
    /// adapter that reached for its own collaborator could not be constructed two ways, and
    /// this one is constructed by a suite that has no sink at all.
    pub fn set_notification_sink(&self, sink: Arc<dyn NotificationSink>) {
        *self.notifications.lock().expect("notification sink lock") = sink;
    }

    pub fn with_askpass(&self, channel: Arc<AskpassChannel>, available: bool) {
        *self.askpass.lock().expect("askpass lock") = Some(channel);
        self.assisted_available
            .store(available, std::sync::atomic::Ordering::SeqCst);
    }

    /// Why the last connection ended, if it has ended (§3.4).
    ///
    /// `None` while a child is running: a connection that has not failed has no condition,
    /// and inventing one would make every caller check a sentinel.
    pub fn last_failure(&self) -> Option<FailureCondition> {
        let code = (*self.exit.lock().expect("exit lock"))?;
        Some(classify(code, &self.stderr_tail()))
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
        self.connect_with(&self.spec)
    }

    fn connect_with(&self, spec: &SpawnSpec) -> Result<(), SpawnError> {
        let _ = self.state_tx.send(ConnectionState::Connecting);
        // A new attempt starts with no memory of the last one's ending, or the first
        // `last_failure()` after a successful reconnect would report the previous failure.
        *self.exit.lock().expect("exit lock") = None;
        self.stderr.lock().expect("stderr lock").clear();

        let child = match self.spawner.spawn(spec) {
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
            let notifications = self.notifications.lock().expect("sink lock").clone();
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
                            Ok(Some(frame)) => {
                                deliver(&registry, notifications.as_ref(), &frame.0)
                            }
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
            let exit = self.exit.clone();
            let mut stderr = child.stderr;
            let wait = child.wait;
            threads.push(std::thread::spawn(move || {
                let mut text = String::new();
                let _ = stderr.read_to_string(&mut text);
                {
                    let mut guard = sink.lock().expect("stderr lock");
                    guard.push_str(&text);
                    if guard.len() > MAX_STDERR_BYTES {
                        let cut = guard.len() - MAX_STDERR_BYTES;
                        *guard = guard[cut..].to_string();
                    }
                }
                // Stderr reaching EOF and the child exiting are the same moment, so the
                // reap happens here rather than in a thread of its own. It also means the
                // exit code is never published before the stderr that explains it.
                let code = wait();
                *exit.lock().expect("exit lock") = code;
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

/// The transport as the connect sequence sees it: one attempt, in one phase, with a
/// classified ending. Keeping this impl here rather than in the use case is what lets the
/// sequence be tested without a process at all.
impl ConnectAttempt for SshTransport {
    async fn attempt(&self, assisted: bool) -> Result<(), FailureCondition> {
        let mut spec = self.spec.clone();
        spec.assisted = assisted;

        if let Err(e) = self.connect_with(&spec) {
            // The child never started. That is not a classified remote condition — it is a
            // local one — so it is reported as `Unknown` rather than guessed into a remedy
            // the user cannot act on.
            crate::logging::warn(&format!("could not start ssh: {e}"));
            return Err(FailureCondition::Unknown);
        }

        // Watch for an early exit. Polling rather than a condition variable because the
        // waiting is bounded and one allocation-free loop is easier to reason about than a
        // second synchronisation primitive shared with three threads.
        let deadline = std::time::Instant::now() + SETTLE;
        while std::time::Instant::now() < deadline {
            if let Some(condition) = self.last_failure() {
                self.shutdown();
                return Err(condition);
            }
            // `tokio::time::sleep`, not the thread's: this runs on the interaction path,
            // and blocking the runtime for the settle period would freeze the window it is
            // reporting to.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }

    fn arm(&self, secret: Secret) {
        match self.askpass.lock().expect("askpass lock").as_ref() {
            Some(channel) => channel.arm(secret),
            // Dropping the secret here zeroes it. An assisted attempt with nowhere to send
            // a passphrase is refused by the spawner, so this path ends in a clean failure
            // rather than a prompt that hangs.
            None => crate::logging::warn("a passphrase was offered with no askpass channel"),
        }
    }

    fn assisted_available(&self) -> bool {
        self.assisted_available
            .load(std::sync::atomic::Ordering::SeqCst)
            && self.askpass.lock().expect("askpass lock").is_some()
    }
}

/// The transport as the status bar sees it (F000's port, unchanged).
///
/// This impl is the swap Principle VIII was for: the interface layer, the use case and the
/// port are untouched, and the composition root binds a different adapter. If this had
/// required changing `ObserveConnection` or `StatusBar.svelte`, the boundary would have
/// been decorative.
impl ConnectionStatusSource for SshTransport {
    fn current(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    fn subscribe(&self, sink: StateSink) {
        // Invoked once immediately, so a subscriber never polls for its initial value.
        sink(self.current());

        // A thread with its own single-threaded runtime rather than a `tokio::spawn`,
        // because this is called from the composition root during startup, which is not
        // guaranteed to be inside a runtime — and a `spawn` that panics there would take
        // the status bar down with it.
        let mut rx = self.state_rx.clone();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            else {
                crate::logging::warn("could not watch connection state: no runtime");
                return;
            };
            rt.block_on(async move {
                while rx.changed().await.is_ok() {
                    let state = *rx.borrow_and_update();
                    sink(state);
                }
            });
        });
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
fn deliver(registry: &Registry, notifications: &dyn NotificationSink, body: &str) {
    let Some(id) = reply_id(body) else {
        // No id, so there is nothing to correlate -- but it is not nothing. An engine-initiated
        // frame is a task's output, its exit, or a file event, and dropping it here is what
        // made a terminal that renders correctly and shows nothing.
        if let Some(method) = extract_string(body, "\"method\"") {
            notifications.deliver(&method, body);
        }
        return;
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
        let (_id, pending) = self.begin(request);
        pending.await
    }

    fn begin(&self, request: Request) -> (RequestId, Pending) {
        // Registration precedes transmission (FR-011), so a reply arriving the instant the
        // write completes is still matched. The other order leaves a window in which a
        // correct reply is discarded as unknown, and the request then times out — a bug
        // that appears only under load and looks like a slow engine.
        let (id, awaiting) = self.registry.register();
        let registry = self.registry.clone();

        let ready = |registry: &Arc<Registry>, id: &RequestId, outcome: RequestOutcome| {
            // Clear the entry: a request that never reached the wire must not be retained.
            registry.resolve(id, RequestOutcome::Withdrawn);
            let out = outcome;
            (id.clone(), Box::pin(async move { out }) as Pending)
        };

        let body = exchange::request_body(&id, &request);
        let frame = match FrameCodec::new().encode(&body) {
            Ok(f) => f,
            // Refused before transmission. An oversized frame is never partially written,
            // so the stream stays aligned and the caller learns the cap rather than
            // watching everything behind it stall.
            Err(FrameError::TooLarge(n)) => {
                return ready(
                    &registry,
                    &id,
                    RequestOutcome::Failed {
                        code: ERR_PAYLOAD_TOO_LARGE,
                        message: format!("payload of {n} bytes exceeds the frame limit"),
                    },
                )
            }
            Err(FrameError::Malformed(why)) => {
                return ready(
                    &registry,
                    &id,
                    RequestOutcome::Failed {
                        code: ERR_INTERNAL,
                        message: why,
                    },
                )
            }
        };

        {
            let guard = self.live.lock().expect("live lock");
            let Some(live) = guard.as_ref() else {
                // Never queue for a connection that may never return.
                return ready(&registry, &id, RequestOutcome::ConnectionLost);
            };
            live.queue.push(request.priority, frame);
        }

        let limit = exchange::deadline_for(&request);
        let for_timeout = id.clone();
        let pending: Pending = Box::pin(async move {
            match tokio::time::timeout(limit, awaiting).await {
                Ok(Ok(outcome)) => outcome,
                // The sender was dropped without sending: the connection went away.
                Ok(Err(_)) => RequestOutcome::ConnectionLost,
                Err(_) => {
                    // Remove the entry: a timed-out request must stop occupying the
                    // registry whether or not a late reply ever arrives.
                    registry.resolve(&for_timeout, RequestOutcome::TimedOut);
                    RequestOutcome::TimedOut
                }
            }
        });
        (id, pending)
    }

    fn withdraw(&self, id: &RequestId) {
        // §4.5: tell the remote side so it can stop work. Best effort by design — the
        // caller is released regardless.
        if let Ok(guard) = self.live.lock() {
            if let Some(live) = guard.as_ref() {
                let body = exchange::cancellation_body(id);
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
