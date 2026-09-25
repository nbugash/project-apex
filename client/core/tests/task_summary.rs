//! FR-032: what the developer is told after a reconnection.
//!
//! Taken from the **per-task results** -- `running`, `retained`, `exitCode`, `signal` -- and never
//! inferred from whether a panel resumed. A panel that resumed says the client managed to attach;
//! it says nothing about whether the build passed, and a summary built from it would be confident
//! and wrong in exactly the case the developer cares about.

mod common;

use apex_protocol::wire::{SignalName, TaskId};
use apex_shell::application::use_cases::observe_connection::{reattach, Outcome};
use apex_shell::application::use_cases::observe_task::Ending;
use apex_shell::domain::workspace::WorkspaceId;
use common::reconnect::{finished, running, signalled, Recorder};

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

#[tokio::test]
async fn the_summary_separates_survived_finished_and_gone() {
    // All three at once, because the interesting reconnection is the mixed one: a long build
    // still going, a short one that finished, and one the engine has forgotten.
    let r = Recorder::default();
    r.answer("still-going", running(8192));
    r.answer("finished-ok", finished(0));
    r.answer("finished-bad", finished(101));
    r.answer("killed", signalled("SIGKILL"));

    let report = reattach(
        &ws(),
        &[],
        &[
            TaskId("still-going".into()),
            TaskId("finished-ok".into()),
            TaskId("finished-bad".into()),
            TaskId("killed".into()),
            TaskId("forgotten".into()),
        ],
        (120, 40),
        &r,
        &r,
    )
    .await;

    assert_eq!(
        report.outcomes,
        vec![
            Outcome::Survived {
                task: TaskId("still-going".into()),
                retained: 8192,
            },
            Outcome::Finished {
                task: TaskId("finished-ok".into()),
                ending: Ending::Code(0),
            },
            Outcome::Finished {
                task: TaskId("finished-bad".into()),
                ending: Ending::Code(101),
            },
            Outcome::Finished {
                task: TaskId("killed".into()),
                ending: Ending::Signal(SignalName("SIGKILL".into())),
            },
            Outcome::Gone {
                task: TaskId("forgotten".into()),
            },
        ]
    );
}

#[tokio::test]
async fn how_a_task_finished_is_reported_and_not_reduced_to_whether_it_finished() {
    // A summary saying "two finished" is a summary that made the developer go and look. Zero and
    // 101 are different news, and the distinction is already on the wire.
    let r = Recorder::default();
    r.answer("ok", finished(0));
    r.answer("bad", finished(101));

    let report = reattach(
        &ws(),
        &[],
        &[TaskId("ok".into()), TaskId("bad".into())],
        (80, 24),
        &r,
        &r,
    )
    .await;

    let endings: Vec<&Ending> = report
        .outcomes
        .iter()
        .filter_map(|o| match o {
            Outcome::Finished { ending, .. } => Some(ending),
            _ => None,
        })
        .collect();
    assert_eq!(endings, vec![&Ending::Code(0), &Ending::Code(101)]);
}

#[tokio::test]
async fn what_was_missed_is_summed_across_the_tasks_that_survived() {
    // The number the developer is told. Only surviving tasks have missed anything: a task that
    // finished delivered its last bytes with its ending, and a task the engine forgot has no
    // bytes to account for.
    let r = Recorder::default();
    r.answer("a", running(1000));
    r.answer("b", running(2000));
    r.answer("c", finished(0));

    let report = reattach(
        &ws(),
        &[],
        &[
            TaskId("a".into()),
            TaskId("b".into()),
            TaskId("c".into()),
            TaskId("gone".into()),
        ],
        (80, 24),
        &r,
        &r,
    )
    .await;

    assert_eq!(report.retained(), 3000, "the missed byte count is wrong");
}

#[tokio::test]
async fn a_reconnection_with_nothing_to_report_reports_nothing() {
    // The ordinary case. A client with no tasks reconnects and the developer is told nothing,
    // which is different from being told everything is fine about tasks that do not exist.
    let r = Recorder::default();
    let report = reattach(&ws(), &[], &[], (80, 24), &r, &r).await;
    assert!(report.outcomes.is_empty());
    assert_eq!(report.retained(), 0);
}

#[tokio::test]
async fn an_ending_with_both_fields_or_neither_is_reported_as_unintelligible() {
    // §4.8 carries one or the other. A client that picked whichever was present would read a
    // signalled death as `exitCode` 0 and tell the developer their build passed.
    let r = Recorder::default();
    r.answer("confused", common::reconnect::both_fields());

    let report = reattach(&ws(), &[], &[TaskId("confused".into())], (80, 24), &r, &r).await;
    assert_eq!(
        report.outcomes,
        vec![Outcome::Finished {
            task: TaskId("confused".into()),
            ending: Ending::Unintelligible,
        }]
    );
}
