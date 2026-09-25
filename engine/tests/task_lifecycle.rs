//! A `runTask` frame becomes a process, and its output becomes frames (T055, T057).
//!
//! The MVP path end to end through `dispatch`, so the wiring is asserted rather than assumed:
//! everything below has been tested in isolation, and this is the test that fails if the pieces
//! are correct and not connected.

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::ports::task_runner::Exit;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::Stream;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use common::fake_clock::FakeClock;
use common::fake_runner::{FakeRunner, Script, Step};
use common::FakeFileSystem;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("sink").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn body(id: &str, task: &str, cwd: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": "execution/runTask",
        "params": {
            "workspace_id": "ws-1",
            "task_id": task,
            "command": ["cargo", "test"],
            "cwd": cwd,
            "pty": true
        }
    })
    .to_string()
}

struct Harness {
    registry: SessionRegistry,
    roots: InMemoryRoots,
    fs: Arc<FakeFileSystem>,
    service: TaskService,
    codec: FrameCodec,
    sink: Sink,
    clock: Arc<FakeClock>,
}

fn harness(script: Script) -> Harness {
    let fs = Arc::new(FakeFileSystem::new());
    fs.dir("/w");
    let roots = InMemoryRoots::new(fs.clone());
    roots.register("ws-1", "/w").expect("register");

    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let clock = Arc::new(FakeClock::new());
    let runner = Arc::new(FakeRunner::new());
    runner.script(script);

    Harness {
        registry: SessionRegistry::new(),
        roots,
        fs,
        service: TaskService::new(writer, Arc::clone(&clock) as Arc<_>, runner as Arc<_>),
        codec: FrameCodec,
        sink,
        clock,
    }
}

fn run(h: &Harness, body: &str) -> Action {
    dispatch(
        &h.registry,
        &h.roots,
        h.fs.as_ref(),
        None,
        Some(&h.service),
        &h.codec,
        body,
    )
}

fn wire(h: &Harness) -> String {
    String::from_utf8_lossy(&h.sink.0.lock().expect("sink")).into_owned()
}

#[test]
fn a_run_task_frame_starts_a_task_and_answers_with_its_pid() {
    let h = harness(Script::of(vec![Step::Bytes(
        Stream::Stdout,
        b"compiling".to_vec(),
    )]));

    let Action::Reply(reply) = run(&h, &body("1", "build", ".")) else {
        panic!("runTask was not answered");
    };
    let text = String::from_utf8(reply).expect("utf8");
    assert!(text.contains("\"pid\""), "no pid in the reply: {text}");
    assert!(!text.contains("\"error\""), "{text}");

    // And the output follows on the wire, from the reader thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        h.clock.advance(100);
        if wire(&h).contains("execution/onStdout") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let seen = wire(&h);
    assert!(
        seen.contains("execution/onStdout"),
        "the task's output never reached the wire: {seen}"
    );
    // Base64, so a task's bytes survive a wire that carries text.
    assert!(
        seen.contains(&apex_protocol::base64::encode(b"compiling")),
        "the output was not the bytes the task wrote: {seen}"
    );
}

#[test]
fn a_second_task_under_a_live_identity_is_refused_distinguishably() {
    let h = harness(Script::of(vec![Step::Idle; 100]));
    let _ = run(&h, &body("1", "build", "."));

    let Action::Reply(reply) = run(&h, &body("2", "build", ".")) else {
        panic!("the second runTask was not answered");
    };
    let text = String::from_utf8(reply).expect("utf8");
    // -32010, not -32006. The client's answer is to attach, and the nearest existing code means
    // the opposite (FR-031c, SC-022).
    assert!(text.contains("-32010"), "wrong refusal: {text}");
    assert!(
        text.contains("attach"),
        "the message should tell the client what to do instead: {text}"
    );
}

#[test]
fn a_cwd_outside_the_workspace_is_refused_before_anything_runs() {
    let h = harness(Script::of(vec![]));
    let Action::Reply(reply) = run(&h, &body("1", "build", "../outside")) else {
        panic!("not answered");
    };
    let text = String::from_utf8(reply).expect("utf8");
    assert!(text.contains("-32002"), "{text}");
    assert!(
        !wire(&h).contains("execution/onStdout"),
        "a refused task produced output"
    );
}

#[test]
fn an_engine_without_a_task_service_refuses_rather_than_appearing_to_succeed() {
    let h = harness(Script::of(vec![]));
    let action = dispatch(
        &h.registry,
        &h.roots,
        h.fs.as_ref(),
        None,
        None, // no service composed, as on a non-Linux host
        &h.codec,
        &body("1", "build", "."),
    );
    let Action::Reply(reply) = action else {
        panic!("not answered");
    };
    let text = String::from_utf8(reply).expect("utf8");
    // F004's degradation shape: the loss is stated rather than silent (FR-027, A-WATCHLOCAL).
    assert!(text.contains("-32011"), "{text}");
    assert!(text.contains("no task service"), "{text}");
}

/// The `execution/onExit` frame's params, as raw JSON.
///
/// Raw, because the question is which **key is present**, and any typed struct answers that with
/// `None` for both "absent" and "null". A client reading `exitCode` from a signalled death gets
/// `null` or `0` depending on the serialiser, and both read as success -- which is the confusion
/// §9 spends a second field preventing.
fn exit_params(h: &Harness) -> serde_json::Value {
    let text = wire(h);
    let mut rest = text.as_str();
    while let Some(at) = rest.find("\r\n\r\n") {
        let header = &rest[..at];
        let len: usize = header
            .rsplit(' ')
            .next()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0);
        let start = at + 4;
        if start + len > rest.len() {
            break;
        }
        let body = &rest[start..start + len];
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
            if v["method"] == "execution/onExit" {
                return v["params"].clone();
            }
        }
        rest = &rest[start + len..];
    }
    panic!("no execution/onExit frame reached the wire: {text}");
}

/// Start a task and let it end, advancing the fake clock so the chunker's bound expires.
fn run_to_exit(h: &Harness) {
    let _ = run(h, &body("1", "build", "."));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        h.clock.advance(100);
        if wire(h).contains("execution/onExit") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the task never ended: {}", wire(h));
}

#[test]
fn a_task_that_exits_seven_reports_seven_and_carries_no_signal() {
    // SC-010. Seven, not "non-zero": a build tool's exit code is information, and collapsing it
    // to a boolean throws away what the developer actually needs to see.
    let h = harness(Script::of(vec![]).exiting_with(Exit::Code(7)));
    run_to_exit(&h);
    let params = exit_params(&h);

    assert_eq!(params["exit_code"], 7, "{params}");
    assert!(
        params.get("signal").is_none(),
        "an ordinary exit carried a signal key: {params}"
    );
}

#[test]
fn a_signalled_task_reports_a_name_and_carries_no_exit_code() {
    // §9's other half. The name, never the number: the wire spends a field keeping the
    // distinction that 128 + n throws away, and 143 is otherwise ambiguous between a stop the
    // developer asked for and a program that chose to exit 143.
    let h = harness(Script::of(vec![]).exiting_with(Exit::Signal(9)));
    run_to_exit(&h);
    let params = exit_params(&h);

    assert_eq!(params["signal"], "SIGKILL", "{params}");
    assert!(
        params.get("exit_code").is_none(),
        "a signalled death carried an exit code: {params}"
    );
}

#[test]
fn a_zero_exit_is_still_a_code_and_not_an_absence() {
    // The case an `Option` serialiser gets wrong for free: skipping a field when it is zero
    // makes success indistinguishable from a signalled death at the client.
    let h = harness(Script::of(vec![]).exiting_with(Exit::Code(0)));
    run_to_exit(&h);
    let params = exit_params(&h);

    assert_eq!(params["exit_code"], 0, "{params}");
    assert!(params.get("signal").is_none(), "{params}");
}

#[test]
fn every_ending_carries_exactly_one_of_the_two_fields() {
    // Stated as a property rather than three examples, because the failure it guards against is
    // a fourth case somebody adds later: an ending with both keys, or neither, is a protocol
    // error and there is no sensible way for a client to read one.
    for exit in [
        Exit::Code(0),
        Exit::Code(7),
        Exit::Signal(9),
        Exit::Signal(15),
    ] {
        let h = harness(Script::of(vec![]).exiting_with(exit));
        run_to_exit(&h);
        let params = exit_params(&h);
        let present = usize::from(params.get("exit_code").is_some())
            + usize::from(params.get("signal").is_some());
        assert_eq!(
            present, 1,
            "an ending for {exit:?} carried {present} of the two fields: {params}"
        );
    }
}
