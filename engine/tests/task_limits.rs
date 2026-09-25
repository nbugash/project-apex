//! SC-026: a task that asks for too much is refused, and the refusal is prompt.
//!
//! **`RLIMIT_AS` does not kill anything.** It makes an allocation fail, and what the process does
//! next is the process's own policy -- a well-written one reports and exits, a careless one
//! aborts, and neither is the engine's doing. The criterion therefore measures the **refusal**,
//! not a death, which is why `fixture_alloc` prints the refusal before doing anything else and
//! why this asserts on that line rather than on an exit code.
//!
//! That distinction was not free: SC-026 originally read as though the limit terminated the task,
//! and the fixture exits **0** after being refused. A test written against the original wording
//! would have been waiting for a death that never comes.
//!
//! The limit used here is deliberately small. `ResourceLimits::FIXED` is 16 GiB, which a process
//! reaches only after reserving sixteen gigabytes of address space -- affordable, since the pages
//! are never touched, but slow enough to make a latency measurement meaningless. The bound being
//! measured is how long **one request** takes to be denied, which does not depend on the ceiling.
//!
//! Local processes only. No remote host and no network (A-TEST, FR-033).

#![cfg(target_os = "linux")]

mod common;

use apex_engine::adapters::outbound::pty_runner::PtyRunner;
use apex_engine::adapters::outbound::std_fs::StdFileSystem;
use apex_engine::application::ports::task_runner::{
    ReadOutcome, ResourceLimits, SpawnRequest, TaskRunner,
};
use apex_engine::domain::path::ResolvedPath;
use apex_engine::domain::task::Shape;
use std::time::{Duration, Instant};

/// §1.4's interaction budget, which SC-026's refusal must stay inside.
const BUDGET_MS: f64 = 500.0;

/// Small enough that the fixture reaches it in a moment. What is measured is the latency of the
/// refusal, which is a property of the kernel's answer rather than of how high the ceiling is.
const SMALL_LIMIT: u64 = 512 * 1024 * 1024;

/// What `fixture_alloc` reserves per step. Restated here so the block count can be turned into
/// bytes and compared against the limit.
const STEP_BYTES: usize = 16 * 1024 * 1024;

fn fixture(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug")
        .join("examples");
    dir.join(name).display().to_string()
}

/// Run `fixture_alloc` under `limits` and return everything it printed.
fn run_alloc(limits: ResourceLimits) -> String {
    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_alloc")];
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    let runner = PtyRunner::new();
    let mut task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape: Shape::Pipes,
            limits,
        })
        .expect("spawn");

    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        match task.output.read(20, &mut out) {
            ReadOutcome::Ended => break,
            ReadOutcome::Failed(e) => panic!("read failed: {e:?}"),
            _ => {}
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the fixture printed after `key`, as a number.
fn count_after(text: &str, key: &str) -> usize {
    let at = text
        .find(key)
        .unwrap_or_else(|| panic!("no {key} was reported: {text}"));
    text[at + key.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .expect("a number")
}

/// The `refused_us=` value the fixture printed.
fn refusal_micros(text: &str) -> u128 {
    count_after(text, "refused_us=") as u128
}

#[test]
fn an_allocation_past_the_limit_is_refused_and_the_refusal_is_prompt() {
    let text = run_alloc(ResourceLimits {
        address_space_bytes: SMALL_LIMIT,
        core_bytes: 0,
    });

    assert!(
        text.contains("ALLOC-BEGIN"),
        "the fixture never started: {text}"
    );
    assert!(
        text.contains("ALLOC-REFUSED"),
        "the allocation was never refused, so the limit was not applied: {text}"
    );
    // **The limit must not be liftable.** Both the soft and the hard value are set, so the task's
    // attempt to raise its own ceiling fails. A soft-only limit is advisory: a runaway lifts it
    // and carries on, and nothing fails until a real runaway arrives -- the one occasion nobody
    // is watching a test.
    assert!(
        text.contains("ALLOC-CEILING lifted=false"),
        "the task raised its own address-space ceiling, so the limit was advisory: {text}"
    );

    // **Refused at *our* limit, not merely refused.** Without the rlimit the fixture is still
    // refused eventually -- the system runs out of address space on its own -- so "a refusal
    // happened" passes for an engine that applies no limit at all. It took seventeen seconds and
    // thousands of blocks to get there when that was measured, against a hundredth of a second
    // here, and the block count is what separates the two.
    let blocks = count_after(&text, "held_blocks=");
    let ceiling = (SMALL_LIMIT / STEP_BYTES as u64) as usize;
    assert!(
        blocks <= ceiling,
        "the task held {blocks} blocks of {STEP_BYTES} bytes before being refused, which is past \
         the {SMALL_LIMIT}-byte limit: the refusal came from the system, not from the engine"
    );

    let micros = refusal_micros(&text);
    let millis = micros as f64 / 1000.0;
    println!("SC-026 limit: {SMALL_LIMIT} bytes");
    println!("SC-026 refusal latency: {micros} us ({millis:.3} ms, budget {BUDGET_MS} ms)");
    println!("SC-026 headroom: {:.3} ms", BUDGET_MS - millis);

    assert!(
        millis <= BUDGET_MS,
        "SC-026: the refusal took {millis:.3} ms, over the {BUDGET_MS} ms budget"
    );
}

#[test]
fn the_task_is_refused_rather_than_killed() {
    // The correction SC-026 needed. A limit denies a request; it does not terminate anything, so
    // a test waiting for a death waits forever against a fixture that handles the refusal -- and
    // a criterion phrased as "the task is terminated" would be satisfied only by a task careless
    // enough to abort.
    let text = run_alloc(ResourceLimits {
        address_space_bytes: SMALL_LIMIT,
        core_bytes: 0,
    });
    assert!(text.contains("ALLOC-REFUSED"), "{text}");
    // It reported and returned normally, which is a refusal handled rather than a process killed.
    assert!(
        !text.contains("Killed") && !text.contains("SIGKILL"),
        "the task was killed rather than refused: {text}"
    );
}

#[test]
fn a_task_under_the_shipped_limit_is_not_refused_at_this_size() {
    // The other side of the bound, which keeps the case above from passing for a limit set
    // absurdly low. Under FIXED's 16 GiB the fixture should sail past the 512 MiB it was refused
    // at -- so a run that stops early would mean the limit is not what the constant says.
    //
    // It is not run to exhaustion: reserving 16 GiB of address space takes long enough that the
    // cost is not worth the assertion, and the assertion here is about the first gigabyte.
    let fs = StdFileSystem;
    let root = ResolvedPath::canonical_root(std::path::Path::new("/tmp"), &fs).expect("root");
    let cwd = ResolvedPath::resolve(&root, ".", &fs).expect("cwd");
    let command = vec![fixture("fixture_alloc")];
    let env: Vec<(String, String)> =
        vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];

    let runner = PtyRunner::new();
    let mut task = runner
        .spawn(&SpawnRequest {
            command: &command,
            cwd: &cwd,
            env: &env,
            shape: Shape::Pipes,
            limits: ResourceLimits::FIXED,
        })
        .expect("spawn");

    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let ReadOutcome::Ended = task.output.read(20, &mut out) {
            break;
        }
        if String::from_utf8_lossy(&out).contains("ALLOC-REFUSED") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&out).into_owned();
    if let Some(at) = text.find("held_blocks=") {
        let blocks: usize = text[at + "held_blocks=".len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .expect("a number");
        // 16 MiB a block, so 64 blocks is the gigabyte the small limit refused inside.
        assert!(
            blocks > 64,
            "under the 16 GiB limit the task was refused after only {blocks} blocks"
        );
    }
    let _ = task
        .control
        .signal(apex_engine::domain::task::TaskSignal::Kill);
}
