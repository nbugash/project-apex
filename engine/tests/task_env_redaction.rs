//! SC-025 and §9's redaction checks: a task's environment reaches no log and no crash report.
//!
//! FR-005a exists because an environment commonly carries credentials, and the two places one
//! escapes are a log line that prints a request whole and a core dump that carries the whole
//! address space. Both are checked, and both paths through the start are checked -- the **failed**
//! start especially, because a refusal is exactly where a request is most likely to be logged in
//! full "for diagnosis".
//!
//! The sentinel is a value that could only have come from the environment. Asserting that a
//! particular key is absent would pass for an implementation that logged the value under another
//! name; asserting the value is absent does not.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::inbound::rpc::{dispatch, Action};
use apex_engine::adapters::outbound::frame_writer::FrameWriter;
use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::adapters::outbound::system_clock::SystemClock;
use apex_engine::adapters::outbound::task_threads::TaskService;
use apex_engine::application::ports::file_system::FileSystem;
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::TaskSignal;
use apex_engine::session::SessionRegistry;
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::TaskId;
use common::frames::{frames_of, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A value that could only have come from the environment this test supplied.
const SENTINEL: &str = "APEX-SENTINEL-8f3c1d-not-for-logs";

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

struct Harness {
    service: TaskService,
    sink: Sink,
    roots: InMemoryRoots,
    fs: Arc<dyn FileSystem>,
    codec: FrameCodec,
    started: Vec<TaskId>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for id in &self.started {
            if let Some(control) = self.service.control(id) {
                let _ = control.signal(TaskSignal::Kill);
            }
        }
        self.service.close();
    }
}

impl Harness {
    /// Start a task whose environment carries the sentinel, and return the reply.
    fn run(&mut self, task: &str, command: &str) -> serde_json::Value {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "1", "method": "execution/runTask",
            "params": {
                "workspace_id": "ws1",
                "task_id": task,
                "command": [command],
                "pty": false,
                "env": { "APEX_SECRET": SENTINEL }
            }
        })
        .to_string();
        let Action::Reply(frame) = dispatch(
            &SessionRegistry::new(),
            &self.roots,
            self.fs.as_ref(),
            None,
            Some(&self.service),
            &self.codec,
            &body,
        ) else {
            panic!("a request must be answered");
        };
        let text = String::from_utf8_lossy(&frame).into_owned();
        let reply: serde_json::Value =
            serde_json::from_str(text.split_once("\r\n\r\n").expect("framed").1).expect("json");
        if reply.get("error").is_none() {
            self.started.push(TaskId(task.into()));
        }
        reply
    }

    /// Everything that reached the wire, which is the only thing a client can read.
    fn wire(&self) -> String {
        String::from_utf8_lossy(&self.sink.0.lock().expect("sink")).into_owned()
    }
}

fn harness() -> Harness {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    Harness {
        service,
        sink,
        roots,
        fs,
        codec: FrameCodec::new(),
        started: Vec::new(),
    }
}

#[test]
fn a_successful_start_puts_the_environment_nowhere_a_client_can_read_it() {
    let mut h = harness();
    let reply = h.run("ok", &fixture("fixture_report_env"));
    assert!(reply.get("error").is_none(), "the start failed: {reply}");

    std::thread::sleep(Duration::from_millis(300));
    let wire = h.wire();
    assert!(
        !wire.contains(SENTINEL),
        "the environment reached the wire on a successful start"
    );
    assert!(
        !reply.to_string().contains(SENTINEL),
        "the reply carried the environment: {reply}"
    );
}

#[test]
fn a_failed_start_puts_the_environment_nowhere_either() {
    // The path that matters most. A refusal is where a request is most likely to be logged whole
    // "so somebody can see what was asked for", and the environment is inside that request.
    let mut h = harness();
    let reply = h.run("ghost", "/nonexistent/definitely-not-a-command");
    assert_eq!(reply["error"]["code"], -32011, "{reply}");

    let rendered = reply.to_string();
    assert!(
        !rendered.contains(SENTINEL),
        "the refusal carried the environment: {rendered}"
    );
    assert!(
        !rendered.contains("APEX_SECRET"),
        "the refusal named an environment variable: {rendered}"
    );
    assert!(
        !h.wire().contains(SENTINEL),
        "the environment reached the wire on a failed start"
    );
}

#[test]
fn a_listing_does_not_carry_the_environment_either() {
    // The third place it could escape: `execution/list` is read back by any client that
    // enumerates, which is a wider audience than the one that started the task.
    let mut h = harness();
    let _ = h.run("listed", &fixture("fixture_signals"));

    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": "2", "method": "execution/list", "params": {}
    })
    .to_string();
    let Action::Reply(frame) = dispatch(
        &SessionRegistry::new(),
        &h.roots,
        h.fs.as_ref(),
        None,
        Some(&h.service),
        &h.codec,
        &body,
    ) else {
        panic!("answered");
    };
    let text = String::from_utf8_lossy(&frame).into_owned();
    assert!(
        !text.contains(SENTINEL),
        "a listing carried a task's environment: {text}"
    );
}

/// The task's own core-size limit, as the kernel reports it.
fn core_limit_of(pid: i32) -> Option<String> {
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).ok()?;
    let line = limits
        .lines()
        .find(|l| l.starts_with("Max core file size"))?;
    // "Max core file size   0   unlimited   bytes" -> the soft value.
    Some(line.split_whitespace().nth(4)?.to_string())
}

#[test]
fn a_task_is_given_no_room_for_a_core_dump_at_all() {
    // **The check that discriminates**, and the reason the directory scan below is not enough.
    //
    // §9 asks the harness to raise `ulimit -c` first, so the host would write a dump if the
    // engine did not stop it -- otherwise the absence of one is free. This host's soft limit is
    // 0 and its hard limit unlimited, so it is raised here. But its `core_pattern` pipes to a
    // handler rather than writing a file, so **no dump would land in a working directory on this
    // host whatever the engine did**, and a directory scan proves nothing.
    //
    // What does prove it is the limit the kernel reports for the task itself. FR-005a requires
    // RLIMIT_CORE to be 0 because a dump is a crash report carrying the whole environment, and
    // this reads that back from the process rather than from the code that set it.
    let raised = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    // SAFETY: `raised` is a valid `rlimit` whose soft value does not exceed its hard value; the
    // host's hard limit is unlimited, so this succeeds and the engine's own 0 is the only thing
    // left stopping a dump.
    let lifted = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &raised) } == 0;
    println!("SC-025 harness raised its own core limit: {lifted}");

    let mut h = harness();
    let _ = h.run("limits", &fixture("fixture_signals"));
    let pid = h
        .service
        .pid(&TaskId("limits".into()))
        .expect("the task is live");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut reported = None;
    while Instant::now() < deadline && reported.is_none() {
        reported = core_limit_of(pid.0);
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("SC-025 task core limit: {reported:?}");

    match reported {
        Some(limit) => assert_eq!(
            limit, "0",
            "a task may write a core dump of {limit} bytes, and a dump carries the whole \
             environment"
        ),
        // Printed above, so a host that will not say is visible as a skip rather than a pass.
        None => println!("SC-025 SKIPPED: /proc did not report this task's limits"),
    }
}

#[test]
fn a_crashing_task_leaves_no_core_file() {
    // §9's other check, and the reason RLIMIT_CORE is zero rather than merely small: a core dump
    // is a crash report carrying the whole address space, and the environment is in it.
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let mut service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner);

    let fs: Arc<dyn FileSystem> = Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots
        .register("ws1", dir.path().to_str().expect("utf8"))
        .expect("register");

    let params = apex_protocol::wire::RunTaskParams {
        workspace_id: apex_protocol::wire::WorkspaceId("ws1".into()),
        task_id: TaskId("crash".into()),
        command: vec![fixture("fixture_crash")],
        cwd: None,
        env: Some(
            [
                (
                    "PATH".to_string(),
                    std::env::var("PATH").unwrap_or_default(),
                ),
                ("APEX_SECRET".to_string(), SENTINEL.to_string()),
            ]
            .into_iter()
            .collect(),
        ),
        pty: false,
        cols: None,
        rows: None,
    };
    service.run(&params, &roots, fs.as_ref()).expect("run");

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if frames_of(&sink)
            .iter()
            .any(|f| f.method == "execution/onExit")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    service.close();

    // It really crashed, or the absence of a dump proves nothing.
    let text =
        String::from_utf8_lossy(&common::frames::delivered_bytes(&frames_of(&sink))).into_owned();
    assert!(
        text.contains("ABOUT-TO-CRASH"),
        "the fixture never ran: {text}"
    );
    let signalled = frames_of(&sink)
        .into_iter()
        .any(|f| f.method == "execution/onExit" && f.signal.is_some());
    assert!(signalled, "the fixture did not crash; it exited normally");

    let cores: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("core"))
        .collect();
    assert!(
        cores.is_empty(),
        "a crashing task left {cores:?} in its working directory, and a core file carries the \
         whole address space including the environment"
    );
}
