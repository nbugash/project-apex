//! SC-019, SC-020 and SC-022: coming back to a build that kept going.
//!
//! The ordering claim is the one with teeth, and it is checked as a **sequence, not as a set**.
//! "The missed bytes are present" passes for an implementation that appends them after the live
//! stream, which puts the middle of a build below its end -- a developer reading that sees a
//! compile error after the summary that says the compile succeeded.
//!
//! `retained` is counted **at the moment the drain begins**, not as a high-water mark. A client
//! uses it to say how much it missed; a high-water mark would keep reporting the worst moment of
//! a connection that has since recovered.
//!
//! Driven against `FakeRunner`, so "while detached" is a step rather than a race.

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::ports::task_runner::Exit;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::Stream;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use common::fake_clock::FakeClock;
use common::fake_runner::{FakeRunner, Script, Step};
use common::frames::{frames_of, Frame, Sink};
use common::FakeFileSystem;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Bytes, then enough idle reads for the chunker's time bound to expire and the frame to go out.
///
/// A reader drains only when a read **returns**, so a write followed immediately by a `Block`
/// never reaches the wire: the reader parks holding it. Idling is what a real task does between
/// writes, and it is what gives the chunker its chance.
fn written(stream: Stream, bytes: &[u8]) -> Vec<Step> {
    vec![
        Step::Bytes(stream, bytes.to_vec()),
        Step::Idle,
        Step::Idle,
        Step::Idle,
    ]
}

struct Harness {
    service: TaskService,
    runner: Arc<FakeRunner>,
    /// Whether a task was ever started, so teardown does not reach for a control that was never
    /// created -- a case that only attaches to an identity nobody ran has no control at all.
    started: bool,
    sink: Sink,
    clock: Arc<FakeClock>,
    roots: InMemoryRoots,
    fs: Arc<dyn FileSystem>,
    codec: FrameCodec,
}

impl Drop for Harness {
    /// Let any parked reader go before anything joins on it.
    ///
    /// A case that leaves the script gated -- which most of these do, deliberately -- would
    /// otherwise hang in `close`, because closing joins reader threads and one of them is waiting
    /// for a permit no test is going to grant. Several permits, because a case may have left more
    /// than one `Block` unspent, and a permit nobody uses costs nothing.
    fn drop(&mut self) {
        if self.started {
            for _ in 0..8 {
                self.runner.control(0).release_read();
            }
        }
        self.service.reattach();
        self.service.close();
    }
}

impl Harness {
    fn call(&self, body: &str) -> serde_json::Value {
        let Action::Reply(frame) = dispatch(
            &SessionRegistry::new(),
            &self.roots,
            self.fs.as_ref(),
            None,
            Some(&self.service),
            &self.codec,
            body,
        ) else {
            panic!("a request must be answered");
        };
        let text = String::from_utf8_lossy(&frame).into_owned();
        serde_json::from_str(text.split_once("\r\n\r\n").expect("framed").1).expect("json")
    }

    fn start(&mut self, task: &str, pty: bool) {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "1", "method": "execution/runTask",
            "params": { "workspace_id": "ws1", "task_id": task, "command": ["cargo"], "pty": pty }
        })
        .to_string();
        let reply = self.call(&body);
        assert!(reply.get("error").is_none(), "start failed: {reply}");
        self.started = true;
    }

    fn attach(&self, task: &str) -> serde_json::Value {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "2", "method": "execution/attach",
            "params": { "workspace_id": "ws1", "task_id": task }
        })
        .to_string();
        self.call(&body)
    }

    fn frames(&self) -> Vec<Frame> {
        frames_of(&self.sink)
    }

    /// Every output byte on the wire, in frame order.
    fn wire(&self) -> String {
        let mut out = String::new();
        for f in self.frames() {
            if let Some(d) = f.data {
                out.push_str(&String::from_utf8_lossy(&d));
            }
        }
        out
    }

    /// Let the reader get as far as it can, advancing the fake clock so the chunker's bound fires.
    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            self.clock.advance(100);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Settle until `ready` holds, or give up. Waiting on the condition rather than on a duration
    /// keeps these from being about how fast this machine happens to be.
    fn settle_until(&self, mut ready: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            self.clock.advance(100);
            if ready() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }
}

fn harness(script: Script) -> Harness {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let clock: Arc<FakeClock> = Arc::new(FakeClock::new());
    let runner = Arc::new(FakeRunner::new());
    runner.script(script);
    // An idle read advances this clock, so the chunker time bound expires without the test having
    // to guess how fast the reader spins.
    runner.driving(Arc::clone(&clock));
    let service = TaskService::new(
        writer,
        Arc::clone(&clock) as Arc<_>,
        Arc::clone(&runner) as Arc<_>,
    );

    let fs_impl = Arc::new(FakeFileSystem::new());
    fs_impl.dir("/w");
    let fs: Arc<dyn FileSystem> = fs_impl.clone();
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/w").expect("register");

    Harness {
        service,
        runner,
        started: false,
        sink,
        clock,
        roots,
        fs,
        codec: FrameCodec::new(),
    }
}

#[test]
fn what_was_missed_arrives_before_anything_produced_since() {
    // SC-019 and SC-022, as a **sequence**. BEFORE is written while attached, MISSED while
    // detached, AFTER once the client is back, and the order on the wire must be exactly that.
    //
    // Staged with `Block`, because a fake that hands over every step as fast as it is read would
    // have produced all three before the test could detach -- and the test would then be about a
    // client that never left.
    let mut steps = written(Stream::Stdout, b"BEFORE\n");
    steps.push(Step::Block);
    steps.extend(written(Stream::Stdout, b"MISSED\n"));
    steps.push(Step::Block);
    steps.extend(written(Stream::Stdout, b"AFTER\n"));
    let mut h = harness(Script::of(steps));
    h.start("build", true);
    assert!(
        h.settle_until(|| h.wire().contains("BEFORE")),
        "nothing arrived at all"
    );

    h.service.detach();
    h.runner.control(0).release_read();
    h.settle();
    assert!(
        !h.wire().contains("MISSED"),
        "output reached the wire while nobody was there to read it"
    );

    h.service.reattach();
    assert!(
        h.settle_until(|| h.wire().contains("MISSED")),
        "the replay never arrived"
    );
    h.runner.control(0).release_read();
    assert!(
        h.settle_until(|| h.wire().contains("AFTER")),
        "nothing arrived after the client returned"
    );

    let wire = h.wire();
    let before = wire.find("BEFORE").expect("BEFORE");
    let missed = wire.find("MISSED").expect("MISSED was never replayed");
    let after = wire.find("AFTER").expect("AFTER");
    assert!(
        before < missed && missed < after,
        "the replay is out of order: BEFORE at {before}, MISSED at {missed}, AFTER at {after}"
    );
}

#[test]
fn retained_counts_what_is_about_to_be_drained() {
    // SC-019's number. It is what a client uses to say how much it missed, so it is the size of
    // the drain about to happen -- not a high-water mark, which would keep reporting the worst
    // moment of a connection that has since recovered.
    let mut steps = written(Stream::Stdout, b"aaaaaaaaaa");
    steps.push(Step::Block);
    steps.extend(written(Stream::Stdout, b"bbbbbbbbbb"));
    steps.push(Step::Block);
    let mut h = harness(Script::of(steps));
    h.start("build", true);
    h.settle();

    h.service.detach();
    h.runner.control(0).release_read();
    h.settle();

    let reply = h.attach("build");
    let retained = reply["result"]["retained"].as_u64().expect("retained");
    assert!(
        retained > 0,
        "nothing was reported as retained although output was produced while detached: {reply}"
    );

    // Drain it, then ask again: nothing is outstanding, so the count is zero.
    h.service.reattach();
    h.settle();
    let again = h.attach("build");
    assert_eq!(
        again["result"]["retained"], 0,
        "a second attach reported bytes that had already been delivered: {again}"
    );
}

#[test]
fn attaching_twice_delivers_nothing_twice() {
    // The failure a high-water mark hides: a client that attaches, receives its replay, and
    // attaches again must not receive the replay a second time. A build's output appearing twice
    // is worse than not appearing -- the developer counts errors.
    let mut h = harness(Script::of(vec![
        Step::Bytes(Stream::Stdout, b"ONCE\n".to_vec()),
        Step::Idle,
    ]));
    h.start("build", true);
    h.service.detach();
    h.settle();

    let _ = h.attach("build");
    h.service.reattach();
    h.settle();
    let first = h.wire().matches("ONCE").count();

    h.service.reattach();
    h.settle();
    let second = h.wire().matches("ONCE").count();

    assert_eq!(first, second, "the replay was delivered twice");
    assert_eq!(first, 1, "the line appeared {first} times");
}

#[test]
fn a_task_that_ended_while_detached_says_how_it_ended() {
    // SC-020 and FR-031b. `running: false` alone says only that it is over; a client reattaching
    // to a finished build needs to know **how** -- and exactly one of the two fields, the same
    // one-of-two shape §9 uses everywhere else.
    //
    // The identity has to survive the ending for this to be answerable at all. An engine that
    // released on an ending it could not deliver would answer -32006 here, and the developer
    // would never learn whether the build they left running had passed.
    let mut steps = written(Stream::Stdout, b"done\n");
    steps.push(Step::Block);
    let mut h = harness(Script::of(steps).exiting_with(Exit::Code(7)));
    h.start("build", true);
    h.settle();

    h.service.detach();
    h.runner.control(0).release_read();
    // Wait for the task to have actually ended, rather than for a duration: what is being
    // asserted is what an attach says about a finished task, and attaching too early would be
    // asserting it about a running one.
    assert!(
        h.settle_until(|| h.attach("build")["result"]["running"] == false),
        "the task never ended while detached"
    );

    let reply = h.attach("build");
    let result = &reply["result"];
    assert_eq!(result["running"], false, "{reply}");
    assert_eq!(result["exit_code"], 7, "{reply}");
    assert!(
        result.get("signal").is_none(),
        "an ordinary exit carried a signal: {reply}"
    );
}

#[test]
fn each_chunk_replays_on_its_own_streams_notification() {
    // A `pty: false` task keeps its streams apart, and the replay must too. Collapsing them onto
    // one notification would put a compiler's warnings in with its output, which is the
    // distinction a client asked for when it chose pipes.
    let mut h = harness(Script::of(vec![
        Step::Bytes(Stream::Stdout, b"OUT\n".to_vec()),
        Step::Bytes(Stream::Stderr, b"ERR\n".to_vec()),
        Step::Idle,
    ]));
    h.start("build", false);
    h.service.detach();
    h.settle();
    h.service.reattach();
    h.settle();

    let frames = h.frames();
    let on = |method: &str, needle: &str| {
        frames
            .iter()
            .filter(|f| f.method == method)
            .filter_map(|f| f.data.as_ref())
            .any(|d| String::from_utf8_lossy(d).contains(needle))
    };
    assert!(on("execution/onStdout", "OUT"), "stdout replayed elsewhere");
    assert!(on("execution/onStderr", "ERR"), "stderr replayed elsewhere");
    assert!(
        !on("execution/onStdout", "ERR"),
        "stderr was replayed on the stdout notification"
    );
}

#[test]
fn attaching_to_a_task_another_workspace_owns_is_not_a_missing_task() {
    let mut h = harness(Script::silent_forever());
    h.start("build", true);

    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": "3", "method": "execution/attach",
        "params": { "workspace_id": "other", "task_id": "build" }
    })
    .to_string();
    let reply = h.call(&body);
    assert_eq!(reply["error"]["code"], -32001, "{reply}");
}

#[test]
fn attaching_to_an_identity_the_engine_never_had_is_refused() {
    let h = harness(Script::silent_forever());
    let reply = h.attach("never-started");
    assert_eq!(reply["error"]["code"], -32006, "{reply}");
}
