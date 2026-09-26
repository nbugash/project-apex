//! A real engine's output reaching the client, over the real transport.
//!
//! This is the seam that did not exist. The transport parsed every inbound frame, matched the
//! ones carrying an id to their requests, and **dropped the rest** -- which is every byte a
//! task produces, every file event and every exit. A terminal that rendered perfectly and
//! showed nothing was the visible half of that; the invisible half is that no test could have
//! caught it, because nothing on either side of the gap was wrong on its own terms.
//!
//! So the assertion here is deliberately end to end and deliberately against the real engine
//! rather than a double: the thing under test is whether two correct halves are joined, and a
//! fake on either side would answer a different question.
//!
//! The engine is spawned as a child on this machine (`LocalEngineSpawner`). It is the same
//! binary, speaking the same protocol, with a pipe where the network would be -- which is the
//! whole reason that spawner is an adapter behind `ProcessSpawner` rather than a transport of
//! its own.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apex_shell::adapters::outbound::local_engine::LocalEngineSpawner;
use apex_shell::adapters::outbound::openssh::SshTransport;
use apex_shell::application::ports::notification_sink::NotificationSink;
use apex_shell::application::ports::spawner::SpawnSpec;
use apex_shell::application::ports::transport::{Request, RequestTransport};
use apex_shell::domain::request::RequestOutcome;

/// Every frame the engine sent unasked, in arrival order.
#[derive(Default)]
struct Recorder {
    frames: Mutex<Vec<(String, String)>>,
}

impl NotificationSink for Recorder {
    fn deliver(&self, method: &str, body: &str) {
        self.frames
            .lock()
            .expect("frames")
            .push((method.to_string(), body.to_string()));
    }
}

impl Recorder {
    fn methods(&self) -> Vec<String> {
        self.frames
            .lock()
            .expect("frames")
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    /// Every `data` field carried by `execution/onStdout`, decoded.
    ///
    /// Decoded rather than compared as base64 because the claim is about the bytes a process
    /// wrote, and an assertion on the encoding would pass for an engine that encoded the wrong
    /// bytes consistently.
    fn stdout(&self) -> String {
        let frames = self.frames.lock().expect("frames");
        frames
            .iter()
            .filter(|(m, _)| m == "execution/onStdout")
            .filter_map(|(_, body)| {
                let at = body.find("\"data\":\"")? + "\"data\":\"".len();
                let rest = &body[at..];
                let end = rest.find('"')?;
                apex_protocol::base64::decode(&rest[..end]).ok()
            })
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .collect()
    }
}

fn engine_binary() -> PathBuf {
    // The suite runs from the crate directory; the binary is built into the workspace target.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("../../target/debug/ide-engine")
}

/// Host and user are required by the spawn contract and meaningless to a child process. Stated
/// here once rather than explained at every call.
fn spec() -> SpawnSpec {
    SpawnSpec {
        host: "localhost".into(),
        user: "local".into(),
        assisted: false,
    }
}

async fn call(t: &SshTransport, method: &str, params: &str) -> RequestOutcome {
    t.send(Request::interactive(method, params)).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_s_output_reaches_the_client_instead_of_being_dropped() {
    let binary = engine_binary();
    if !binary.exists() {
        panic!(
            "the engine is not built at {}; run `cargo build -p apex-engine --bins` first",
            binary.display()
        );
    }

    let recorder = Arc::new(Recorder::default());
    let transport = SshTransport::new(Arc::new(LocalEngineSpawner::new(binary)), spec());
    transport.set_notification_sink(recorder.clone());
    transport.connect().expect("the engine did not start");

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let register = call(
        &transport,
        "workspace/register",
        &format!(r#"{{"workspace_id":"ws1","path":"{}"}}"#, root.display()),
    )
    .await;
    assert!(
        matches!(register, RequestOutcome::Answered(_)),
        "the workspace was not registered: {register:?}"
    );

    // A real command, in a real pseudo-terminal, producing bytes this test did not write.
    let started = call(
        &transport,
        "execution/runTask",
        r#"{"workspace_id":"ws1","task_id":"t1","command":["/bin/echo","apex-terminal-works"],"pty":true}"#,
    )
    .await;
    assert!(
        matches!(started, RequestOutcome::Answered(_)),
        "the task did not start: {started:?}"
    );

    // Poll rather than sleep a fixed span: the output is produced as fast as a fork allows,
    // and a fixed wait is either flaky or slow.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if recorder.stdout().contains("apex-terminal-works") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let seen = recorder.stdout();
    assert!(
        seen.contains("apex-terminal-works"),
        "the task's output never reached the client; frames seen: {:?}",
        recorder.methods()
    );

    // And the ending arrives by the same route. Without this the test passes for a transport
    // that forwards output and drops everything else, which is the same defect one field over.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if recorder.methods().iter().any(|m| m == "execution/onExit") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        recorder.methods().iter().any(|m| m == "execution/onExit"),
        "the exit never reached the client; frames seen: {:?}",
        recorder.methods()
    );
}
