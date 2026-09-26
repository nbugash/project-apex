//! FR-019: terminating a task that has already exited succeeds.
//!
//! The window FR-019 is about is narrow and specific: the process is gone, its exit has been
//! **observed but not yet delivered**, and the identity is therefore still live. A client that
//! pressed stop during that window is racing an end **the engine decided**, and reporting failure
//! would make a correct client look broken.
//!
//! Both halves are required. Without the second -- that a terminate **after** the identity is
//! released is `-32006` -- the test passes for an implementation that answers success to every
//! terminate, which is not FR-019 but the absence of an error.
//!
//! Driven against `FakeRunner`, which holds the drain, so the window is a **step** rather than a
//! race. Two terminates fired at a real process and hoped over is a flake, not a check.

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::ports::task_runner::{
    Exit, ResourceLimits, SpawnRequest, TaskRunner,
};
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::{Shape, Stream};
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::{Pid, TaskId};
use common::fake_clock::FakeClock;
use common::fake_runner::{FakeRunner, Script, Step};
use common::frames::{frames_of, Sink};
use common::FakeFileSystem;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    service: TaskService,
    sink: Sink,
    runner: Arc<FakeRunner>,
    roots: InMemoryRoots,
    fs: Arc<dyn FileSystem>,
    codec: FrameCodec,
    clock: Arc<FakeClock>,
    id: TaskId,
}

impl Drop for Harness {
    /// Let the blocked read go before anything joins on it.
    ///
    /// A case that deliberately leaves the reader held -- which is the whole point of the window
    /// -- would otherwise hang in `TaskService::close`, because closing joins reader threads and
    /// this one is waiting for a test that has already finished. Drop runs before the fields are
    /// dropped, so the release happens while the service still exists.
    fn drop(&mut self) {
        self.runner.control(0).release_read();
        self.service.close();
    }
}

impl Harness {
    /// Send `execution/terminate` as a client would, and return the parsed reply.
    fn terminate(&self, signal: &str) -> serde_json::Value {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "execution/terminate",
            "params": { "task_id": self.id.0, "signal": signal }
        })
        .to_string();
        let action = dispatch(
            &SessionRegistry::new(),
            &self.roots,
            self.fs.as_ref(),
            None,
            Some(&self.service),
            &self.codec,
            &body,
        );
        let Action::Reply(frame) = action else {
            panic!("a request must be answered");
        };
        let text = String::from_utf8_lossy(&frame).into_owned();
        let body = text.split_once("\r\n\r\n").expect("a framed reply").1;
        serde_json::from_str(body).expect("json")
    }

    /// Wait until the reader has passed the exit and blocked mid-drain, which is the window.
    ///
    /// Asked of the fake rather than inferred from output: a reader blocked mid-drain flushes
    /// nothing, so there is no frame that says "the window is open" and a test waiting for one
    /// would wait forever for bytes that reader is holding.
    fn wait_for_window(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            self.clock.advance(100);
            if self.runner.control(0).has_exited() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    fn exit_delivered(&self) -> bool {
        frames_of(&self.sink)
            .iter()
            .any(|f| f.method == "execution/onExit")
    }

    fn wait_for_exit(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            self.clock.advance(100);
            if self.exit_delivered() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("the exit was never delivered");
    }
}

/// A task whose process exits while its output is still draining.
///
/// `BytesAfterExit` puts bytes behind the exit, and `Block` holds the reader there until the test
/// lets go. Between those two the process is gone and the identity is live, which is the window.
fn held_in_the_window() -> Harness {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let clock: Arc<FakeClock> = Arc::new(FakeClock::new());
    let runner = Arc::new(FakeRunner::new());
    // The window, as three steps. The task writes; then bytes arrive **after** the process is
    // marked exited, which is what puts it in the state FR-019 is about; then the reader blocks
    // mid-drain and stays there until a test lets go. Blocking before the exit -- which the first
    // version of this did -- is a reader stalled on a *running* task, a different state entirely
    // and one where signalling again is correct.
    runner.script(
        Script::of(vec![
            Step::Bytes(Stream::Stdout, b"compiling\n".to_vec()),
            Step::BytesAfterExit(Stream::Stdout, b"last-line\n".to_vec()),
            Step::Block,
        ])
        .exiting_with(Exit::Code(0)),
    );
    let service = TaskService::new(
        Arc::clone(&writer),
        Arc::clone(&clock) as Arc<_>,
        Arc::clone(&runner) as Arc<_>,
    );

    let fs_impl = Arc::new(FakeFileSystem::new());
    fs_impl.dir("/w");
    let fs: Arc<dyn FileSystem> = fs_impl.clone();
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/w").expect("register");

    let root = ResolvedPath::canonical_root(std::path::Path::new("/w"), fs.as_ref()).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", fs.as_ref()).expect("cwd");
    let command = vec!["cargo".to_string()];
    let env: Vec<(String, String)> = vec![];
    let task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape: Shape::Pipes,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");
    let id = TaskId("build".into());
    service.adopt(id.clone(), Pid(4242), task.control, task.output);

    Harness {
        service,
        sink,
        runner,
        roots,
        fs,
        codec: FrameCodec::new(),
        clock,
        id,
    }
}

#[test]
fn a_second_terminate_inside_the_window_succeeds_and_signals_nothing() {
    let h = held_in_the_window();

    // Get the reader past the exit and into the drain, so the window is entered before anything
    // is asserted about it.
    assert!(
        h.wait_for_window(),
        "the reader never passed the exit, so the window was never entered"
    );

    // The first stop, already inside the window.
    let first = h.terminate("SIGTERM");
    assert!(
        first.get("error").is_none(),
        "the first terminate failed: {first}"
    );
    // Zero, not one: this terminate is already inside the window. The process is gone and its
    // pid is free for the kernel to hand out, so a signal now reaches whatever holds that number
    // next -- and it goes to the whole group, which makes it worse.
    let signals_after_first = h.runner.control(0).signals().len();
    assert_eq!(
        signals_after_first, 0,
        "a terminate inside the window signalled a process that is already gone"
    );

    // The window: the reader is blocked, so the exit is observed and not delivered, and the
    // identity is still live.
    assert!(
        !h.exit_delivered(),
        "the exit was delivered before the drain finished; there is no window to test"
    );

    let second = h.terminate("SIGTERM");
    assert!(
        second.get("error").is_none(),
        "FR-019: a terminate inside the window must succeed, got {second}"
    );
    assert_eq!(
        second["result"],
        serde_json::Value::Null,
        "the reply must be result: null, got {second}"
    );

    // Zero additional signals. There is nothing left to signal, and sending one to a pid that
    // has been reused is how a stop reaches a process nobody asked about.
    assert_eq!(
        h.runner.control(0).signals().len(),
        signals_after_first,
        "a terminate inside the window signalled a process that is already gone"
    );
}

#[test]
fn a_terminate_after_the_identity_is_released_is_refused() {
    let h = held_in_the_window();
    let _ = h.terminate("SIGTERM");

    // Let the drain finish: the blocked read returns, the remaining output goes out, the exit
    // is delivered, and only then is the identity released.
    h.runner.control(0).release_read();
    h.wait_for_exit();

    let after = h.terminate("SIGTERM");
    assert_eq!(
        after["error"]["code"], -32006,
        "a terminate for a released identity must be -32006, got {after}"
    );
}

#[test]
fn the_exit_arrives_after_everything_the_task_wrote() {
    // The ordering the release depends on. If the identity were released before the exit was on
    // the wire, a client that terminated and then attached -- to collect the last of the output
    // -- would find nothing (FR-022, FR-023).
    let h = held_in_the_window();
    h.runner.control(0).release_read();
    h.wait_for_exit();

    let frames = frames_of(&h.sink);
    let exit_at = frames
        .iter()
        .position(|f| f.method == "execution/onExit")
        .expect("an exit");
    let last_output = frames
        .iter()
        .rposition(|f| f.method.starts_with("execution/onStd"))
        .expect("some output");
    assert!(
        last_output < exit_at,
        "the exit overtook the output it should have followed"
    );
}
