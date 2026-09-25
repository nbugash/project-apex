//! SC-009: a resize reaches the process, and how long that takes.
//!
//! The only thing that can answer "did it arrive?" is the process asking the kernel, which is
//! what `fixture_winsize` does -- a `TIOCGWINSZ` on its own descriptor, printed at startup and
//! once per `SIGWINCH`. An engine-side assertion that `ioctl` returned zero would be this process
//! agreeing with itself about a call it made.
//!
//! The measurement is **printed**, not only compared, per the project's non-functional
//! convention: a verdict says a bound held, a number says how much room was left.
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
use apex_engine::application::ports::roots::WorkspaceRoots;
use apex_engine::application::use_cases::task::{
    resize_task, ResizeOutcome, DEFAULT_COLS, DEFAULT_ROWS,
};
use apex_engine::application::use_cases::workspace::InMemoryRoots;
use apex_engine::domain::task::TaskSignal;
use apex_engine::session::SessionRegistry;
use apex_protocol::wire::{RunTaskParams, TaskId, WorkspaceId};
use common::frames::{delivered_bytes, frames_of, Sink};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// SC-009's bound.
const BUDGET_MS: f64 = 500.0;
/// SC-009 asks for at least this many samples.
const MIN_SAMPLES: usize = 100;

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

struct Sized_ {
    service: TaskService,
    sink: Sink,
    id: TaskId,
    /// Held so a resize can be driven through `dispatch`, which is where SC-009's interval
    /// starts. Measuring from `TaskControl::resize` would skip the parse, the lookup and the
    /// use case -- everything between the wire and the syscall -- and report the cost of an
    /// `ioctl` as the cost of a resize.
    roots: InMemoryRoots,
    fs: Arc<dyn apex_engine::application::ports::file_system::FileSystem>,
    codec: apex_protocol::framing::FrameCodec,
}

impl Drop for Sized_ {
    fn drop(&mut self) {
        if let Some(control) = self.service.control(&self.id) {
            let _ = control.signal(TaskSignal::Kill);
        }
        self.service.close();
    }
}

impl Sized_ {
    fn text(&self) -> String {
        String::from_utf8_lossy(&delivered_bytes(&frames_of(&self.sink))).into_owned()
    }

    /// Send `execution/resizePty` exactly as a client would: a notification, with no id.
    fn notify_resize(&self, cols: u16, rows: u16) {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "execution/resizePty",
            "params": { "task_id": self.id.0, "cols": cols, "rows": rows }
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
        // A notification has no reply, and anything else here would be an unsolicited frame.
        assert!(
            matches!(action, Action::Nothing),
            "a notification produced a reply"
        );
    }

    fn wait_for(&self, needle: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }
}

/// Start through `TaskService::run`, which is the path `execution/runTask` takes.
///
/// Not `adopt`: that is the primitive, and it registers the reader without recording the task in
/// the domain's `TaskSet`. The set is what `shape` reads, and a resize has to know whether the
/// task has a terminal at all -- so a test that adopted directly would be asking about a task the
/// engine does not consider to exist, and would conclude the wrong thing about resizing.
fn start(pty: bool, cols: Option<u16>, rows: Option<u16>) -> Sized_ {
    let sink = Sink::default();
    let writer = Arc::new(FrameWriter::new(Box::new(sink.clone())));
    let runner = Arc::new(PtyRunner::new());
    let service = TaskService::new(writer, Arc::new(SystemClock) as Arc<_>, runner.clone());

    let fs: Arc<dyn apex_engine::application::ports::file_system::FileSystem> =
        Arc::new(StdFileSystem);
    let roots = InMemoryRoots::new(Arc::clone(&fs));
    roots.register("ws1", "/tmp").expect("register");

    let id = TaskId("size".into());
    let params = RunTaskParams {
        workspace_id: WorkspaceId("ws1".into()),
        task_id: id.clone(),
        command: vec![fixture("fixture_winsize")],
        cwd: None,
        env: Some(
            [(
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_default(),
            )]
            .into_iter()
            .collect(),
        ),
        pty,
        cols,
        rows,
    };
    service.run(&params, &roots, fs.as_ref()).expect("run");
    Sized_ {
        service,
        sink,
        id,
        roots,
        fs,
        codec: apex_protocol::framing::FrameCodec::new(),
    }
}

#[test]
fn the_size_a_task_is_started_with_is_the_size_it_reports() {
    // US2.3a. Read from the process, not from the engine's record: the record is what the engine
    // intended and the process is what happened.
    let running = start(true, Some(120), Some(40));
    assert!(
        running.wait_for("SIZE cols=120 rows=40", Duration::from_secs(10)),
        "the task did not start at the size it was given: {}",
        running.text()
    );
}

#[test]
fn a_task_given_no_size_starts_at_the_stated_default() {
    // §4.8's default, and specifically not the kernel's 0 x 0 -- a size no display has, and the
    // one value `resizePty` refuses. The constants are read, never restated.
    let running = start(true, None, None);
    assert!(
        running.wait_for(
            &format!("SIZE cols={DEFAULT_COLS} rows={DEFAULT_ROWS}"),
            Duration::from_secs(10)
        ),
        "the default size is not what the task saw: {}",
        running.text()
    );
}

#[test]
fn a_resize_reaches_the_process_within_the_interaction_budget() {
    let running = start(true, Some(80), Some(24));
    assert!(
        running.wait_for("SIZE ", Duration::from_secs(10)),
        "the task never started"
    );
    // Alternating, so each resize is a real change: sending the same size twice produces no
    // SIGWINCH on some kernels and the sample would be of nothing happening.
    let mut samples: Vec<f64> = Vec::new();
    for n in 0..MIN_SAMPLES + 20 {
        let (cols, rows) = if n % 2 == 0 { (100, 30) } else { (90, 28) };
        let needle = format!("RESIZED cols={cols} rows={rows}");
        let before = running.text();
        let sent = Instant::now();
        running.notify_resize(cols, rows);
        let deadline = sent + Duration::from_secs(5);
        let mut seen = false;
        while Instant::now() < deadline {
            let now = running.text();
            // Count only a *new* occurrence: the transcript keeps every earlier line, so
            // matching the whole text would make every sample after the first instantaneous.
            if now.matches(&needle).count() > before.matches(&needle).count() {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            seen,
            "a resize never reached the process: {}",
            running.text()
        );
        samples.push(sent.elapsed().as_secs_f64() * 1000.0);
    }

    assert!(
        samples.len() >= MIN_SAMPLES,
        "SC-009 needs at least {MIN_SAMPLES} samples, measured {}",
        samples.len()
    );
    let mut sorted = samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let rank = ((sorted.len() as f64) * 0.99).ceil() as usize;
    let p99 = sorted[rank.saturating_sub(1).min(sorted.len() - 1)];
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;

    println!("SC-009 samples: {}", samples.len());
    println!("SC-009 min:  {:.3} ms", sorted[0]);
    println!("SC-009 mean: {mean:.3} ms");
    println!("SC-009 p99:  {p99:.3} ms (budget {BUDGET_MS} ms)");
    println!("SC-009 max:  {:.3} ms", sorted[sorted.len() - 1]);
    println!("SC-009 headroom at p99: {:.3} ms", BUDGET_MS - p99);

    assert!(
        p99 <= BUDGET_MS,
        "SC-009 p99 was {p99:.3} ms, over the {BUDGET_MS} ms budget"
    );
}

#[test]
fn a_resize_of_a_task_without_a_terminal_changes_nothing_and_leaves_it_running() {
    // US2.3b. No error frame, no change, and the task keeps going -- a resize for a task with
    // pipes is a client reporting a panel size, not a request that can fail.
    let running = start(false, None, None);
    assert!(
        running.wait_for("SIZE cols=0 rows=0", Duration::from_secs(10)),
        "the pipes task never reported its (absent) size: {}",
        running.text()
    );
    let before = frames_of(&running.sink).len();

    assert_eq!(
        resize_task(
            running.service.shape(&running.id),
            120,
            40,
            running.service.control(&running.id)
        ),
        ResizeOutcome::NoTerminal
    );

    std::thread::sleep(Duration::from_millis(300));
    let frames = frames_of(&running.sink);
    assert!(
        frames.iter().all(|f| f.method != "execution/onExit"),
        "the task ended after a resize it should have ignored"
    );
    assert!(
        !running.text().contains("RESIZED"),
        "a task with pipes reported a resize: {}",
        running.text()
    );
    assert!(
        frames.len() >= before,
        "frames disappeared, which is not a thing that can happen"
    );
}
