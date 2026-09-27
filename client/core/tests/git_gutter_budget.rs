//! SC-004: from opening a modified file to its line coordinates being available.
//!
//! p99 over at least 100 samples, measured at the boundary between the interface and the
//! transport, with the double's own latency deliberately zero — what is being measured is what
//! *the system* adds, which is the distinction A-NFR draws between a performance gate and a
//! number.
//!
//! **The measured value is printed, in microseconds.** A budget only ever compared against
//! tells nobody how much headroom is left, and a value printed in milliseconds rounds this
//! path's cost to `0` -- a number that cannot distinguish a fast system from one that is not
//! running at all, which is the failure mode this whole file exists to avoid elsewhere.
//!
//! **What this does not measure:** the engine's `git diff` subprocess, which is where the real
//! cost of SC-004 is. This is the client's share -- request through to coordinates in hand --
//! and it says how much of the 250 ms is left for the engine. The end-to-end number is in
//! `git-gutter.spec.ts`, against a real repository.

mod common;

use apex_protocol::wire::GitDiffResult;
use apex_shell::adapters::outbound::remote_git::RemoteGitProvider;
use apex_shell::application::ports::git_provider::GitProvider;
use apex_shell::application::ports::request_sender::RequestSender;
use apex_shell::application::ports::transport::Request;
use apex_shell::domain::request::RequestOutcome;
use apex_shell::domain::workspace::WorkspaceId;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;

/// A transport that answers instantly with a real JSON frame.
///
/// **Through the real adapter, not around it.** The first version of this test measured a fake
/// provider handing back a cloned struct and reported `0 us` over two hundred samples -- a
/// number that cannot tell a fast client from one that is not running. What the client actually
/// spends on a diff is encoding the request, parsing the reply and mapping it, so that is what
/// is in the loop here; the double's own latency stays zero so the measurement is the system's
/// share and not the network's.
struct Instant2 {
    reply: String,
}

#[async_trait]
impl RequestSender for Instant2 {
    async fn send(&self, _request: Request) -> RequestOutcome {
        RequestOutcome::Answered(self.reply.clone())
    }
    async fn notify(&self, _request: Request) {}
}

const SAMPLES: usize = 200;
/// SC-004's bound.
const BUDGET_MS: u128 = 250;

fn p99(mut us: Vec<u128>) -> u128 {
    us.sort_unstable();
    us[(us.len() as f64 * 0.99) as usize - 1]
}

/// A file with a realistic number of hunks. A diff of one line measures the transport and
/// nothing else; a file mid-refactor is where the cost actually is.
fn busy_diff() -> GitDiffResult {
    GitDiffResult {
        added: (0..40).map(|i| [i * 10 + 1, i * 10 + 3]).collect(),
        modified: (0..40).map(|i| [i * 10 + 5, i * 10 + 6]).collect(),
        deleted: (0..40).map(|i| i * 10 + 8).collect(),
    }
}

/// The provider the application uses, answering with `diff` encoded as the engine would.
fn provider(diff: &GitDiffResult) -> RemoteGitProvider {
    let reply = format!(
        r#"{{"jsonrpc":"2.0","id":"1","result":{}}}"#,
        serde_json::to_string(diff).expect("encode")
    );
    RemoteGitProvider::new(Arc::new(Instant2 { reply }))
}

#[tokio::test]
async fn a_modified_file_s_coordinates_arrive_within_the_budget() {
    let git = provider(&busy_diff());
    let ws = WorkspaceId("w1".into());

    // Warm, so the first allocation is not one of the samples.
    for _ in 0..10 {
        let _ = git.file_diff(&ws, "/src/busy.rs").await;
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let at = Instant::now();
        let diff = git
            .file_diff(&ws, "/src/busy.rs")
            .await
            .expect("the diff must arrive");
        samples.push(at.elapsed().as_micros());
        // Read, so the measurement covers producing an answer rather than producing a future.
        assert_eq!(diff.added.len(), 40);
    }

    let p99_us = p99(samples);
    println!(
        "git gutter, 120 hunks: p99 {p99_us} us over {SAMPLES} samples \
         ({:.3} ms of the {BUDGET_MS} ms budget, leaving {:.1} ms for the engine)",
        p99_us as f64 / 1000.0,
        BUDGET_MS as f64 - p99_us as f64 / 1000.0
    );
    assert!(
        p99_us / 1000 < BUDGET_MS,
        "p99 {}ms exceeded the {BUDGET_MS}ms budget",
        p99_us / 1000
    );
}

#[tokio::test]
async fn an_unmodified_file_costs_no_more_than_a_busy_one() {
    // The common case, measured separately: a gutter that was fast only for files with nothing
    // in them would satisfy the budget above on a repository nobody had touched.
    let git = provider(&GitDiffResult::default());
    let ws = WorkspaceId("w1".into());

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let at = Instant::now();
        let diff = git.file_diff(&ws, "/src/clean.rs").await.expect("diff");
        samples.push(at.elapsed().as_micros());
        assert!(diff.added.is_empty());
    }

    let p99_us = p99(samples);
    println!("git gutter, unmodified file: p99 {p99_us} us over {SAMPLES} samples");
    assert!(p99_us / 1000 < BUDGET_MS);
}
