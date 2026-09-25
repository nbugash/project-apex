//! The reattachment sequence, in order.
//!
//! **Order is the claim**, so it is asserted as a sequence rather than as a set of calls that all
//! happened. Two of the steps are cheap to get wrong in a way nothing else notices: listing when
//! the identities were never lost spends a round trip learning what the client already knew, on
//! the one path where the developer is waiting and the link has just proved unreliable; and
//! resizing before attaching tells a task a size it will forget when the attach arrives.

mod common;

use apex_protocol::wire::{SignalName, TaskId};
use apex_shell::application::ports::task_provider::{AttachResult, Pid, TaskSummary};
use apex_shell::application::ports::workspace_provider::ProviderError;
use apex_shell::application::use_cases::observe_connection::{reattach, Outcome, Step};
use apex_shell::domain::workspace::WorkspaceId;
use common::reconnect::{finished, running, Recorder};

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

#[tokio::test]
async fn the_sequence_runs_in_order_and_skips_the_listing_it_does_not_need() {
    let r = Recorder::default();
    r.answer("build", running(4096));

    let report = reattach(&ws(), &[], &[TaskId("build".into())], (120, 40), &r, &r).await;

    // **No list.** The identities were remembered, so enumerating would spend a round trip
    // learning what the client already knew.
    assert_eq!(
        r.calls(),
        vec!["watch", "attach:build", "resize:build"],
        "the sequence ran in the wrong order, or listed when it had no need to"
    );
    assert_eq!(
        report.steps,
        vec![
            Step::Watch,
            Step::Attach(TaskId("build".into())),
            Step::Resize(TaskId("build".into())),
        ]
    );
    assert_eq!(report.retained(), 4096);
}

#[tokio::test]
async fn a_client_that_lost_its_identities_lists_to_find_them() {
    // SC-023. This is the only path on which listing is worth a round trip, and it is the
    // recovery path: without it these tasks keep running and are unreachable.
    let r = Recorder::default();
    *r.listed.lock().expect("listed") = vec![TaskSummary {
        task_id: TaskId("found".into()),
        workspace_id: apex_protocol::wire::WorkspaceId("ws1".into()),
        command: vec!["cargo".into()],
        pty: true,
        pid: Pid(7),
        running: true,
        exit_code: None,
        signal: None,
    }];
    r.answer("found", running(0));

    let report = reattach(&ws(), &[], &[], (120, 40), &r, &r).await;

    assert_eq!(
        r.calls(),
        vec!["watch", "list", "attach:found", "resize:found"],
        "a client with nothing remembered must list before it can attach"
    );
    assert!(report.steps.contains(&Step::List));
}

#[tokio::test]
async fn a_released_identity_is_surfaced_rather_than_retried() {
    // -32006 means the identity is released, so attaching again asks the same question and gets
    // the same answer. A retry loop here tells the developer nothing while it waits for a task
    // that is gone.
    let r = Recorder::default();
    r.answer("ghost", Err(ProviderError::TaskNotFound));

    let report = reattach(&ws(), &[], &[TaskId("ghost".into())], (80, 24), &r, &r).await;

    assert_eq!(
        r.calls().iter().filter(|c| c.starts_with("attach")).count(),
        1,
        "the attach was retried"
    );
    assert_eq!(
        report.outcomes,
        vec![Outcome::Gone {
            task: TaskId("ghost".into())
        }]
    );
    // And nothing was resized: there is no terminal behind an identity the engine released.
    assert!(!r.calls().iter().any(|c| c.starts_with("resize")));
}

#[tokio::test]
async fn a_task_that_finished_while_away_is_reported_and_not_resized() {
    // FR-032 and SC-020. The developer is told how it ended; resizing it would be a round trip
    // spent telling a finished process about a window.
    let r = Recorder::default();
    r.answer("done", finished(7));

    let report = reattach(&ws(), &[], &[TaskId("done".into())], (80, 24), &r, &r).await;

    assert_eq!(
        report.outcomes,
        vec![Outcome::Finished {
            task: TaskId("done".into()),
            ending: apex_shell::application::use_cases::observe_task::Ending::Code(7),
        }]
    );
    assert!(
        !r.calls().iter().any(|c| c.starts_with("resize")),
        "a finished task was told its size"
    );
}

#[tokio::test]
async fn every_attached_task_is_told_its_size() {
    // Attaching deliberately sets none (attach guarantee 9), so a client that forgets leaves the
    // task laying out to the width of a window that has since been resized or closed.
    let r = Recorder::default();
    r.answer("a", running(0));
    r.answer("b", running(0));

    let report = reattach(
        &ws(),
        &[],
        &[TaskId("a".into()), TaskId("b".into())],
        (200, 60),
        &r,
        &r,
    )
    .await;

    assert_eq!(
        r.calls(),
        vec!["watch", "attach:a", "resize:a", "attach:b", "resize:b"],
        "a size must follow each attach, not precede it or be batched after both"
    );
    assert_eq!(report.outcomes.len(), 2);
}

#[tokio::test]
async fn a_signalled_ending_keeps_its_name() {
    let r = Recorder::default();
    r.answer(
        "killed",
        Ok(AttachResult {
            pid: Pid(1),
            running: false,
            retained: 0,
            exit_code: None,
            signal: Some(SignalName("SIGKILL".into())),
        }),
    );

    let report = reattach(&ws(), &[], &[TaskId("killed".into())], (80, 24), &r, &r).await;
    assert_eq!(
        report.outcomes,
        vec![Outcome::Finished {
            task: TaskId("killed".into()),
            ending: apex_shell::application::use_cases::observe_task::Ending::Signal(SignalName(
                "SIGKILL".into()
            )),
        }]
    );
}
