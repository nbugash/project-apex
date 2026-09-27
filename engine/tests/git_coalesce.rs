//! When git actually runs, decided against a settable clock.
//!
//! No filesystem and no inotify: the question here is arithmetic, and the same assertions
//! against a real clock would sleep and then be ignored.
//!
//! **What is counted is status computations, not notifications.** A coalescer that emits one
//! notification while running git fifty times satisfies the wrong assertion entirely, and the
//! cost this bounds is the git run, not the message.

use apex_engine::application::use_cases::git_status::{Due, StatusCoalescer, EDGE_MS};

/// A run counted the way the cost is actually incurred: claim, work, finish.
fn run_if_due(c: &mut StatusCoalescer, now: u64, computations: &mut usize) {
    if c.begin(now) {
        *computations += 1;
        c.finish(now);
    }
}

#[test]
fn nothing_pending_runs_nothing() {
    let c = StatusCoalescer::new(EDGE_MS);
    assert_eq!(c.due(0), Due::Idle);
    assert_eq!(c.due(10_000), Due::Idle);
}

#[test]
fn one_change_runs_once_after_the_edge_passes() {
    let mut c = StatusCoalescer::new(EDGE_MS);
    c.notice(0);
    assert_eq!(c.due(0), Due::At(EDGE_MS), "not due before the edge");
    assert_eq!(c.due(EDGE_MS - 1), Due::At(EDGE_MS));
    assert_eq!(c.due(EDGE_MS), Due::Run);
}

#[test]
fn fifty_writes_inside_one_second_cost_one_computation() {
    // FR-006b, SC-013. `git add -A` on a large tree, or a rebase step, writes the index
    // repeatedly; a
    // full-repository status per write is what presents as the client hanging.
    let mut c = StatusCoalescer::new(EDGE_MS);
    let mut computations = 0usize;

    for tick in 0..50u64 {
        c.notice(tick * 20); // every 20 ms, so no gap ever reaches the edge
        run_if_due(&mut c, tick * 20, &mut computations);
    }
    assert_eq!(computations, 0, "nothing should have run during the burst");

    // The burst stops; the edge passes.
    run_if_due(&mut c, 50 * 20 + EDGE_MS, &mut computations);
    assert_eq!(computations, 1, "a settled burst is one computation");
}

#[test]
fn a_burst_of_any_length_costs_two_computations_not_n() {
    // The part a trailing edge alone does not give. A rebase writes the index over *seconds*,
    // so every 100 ms gap would otherwise start another full-repository status.
    let mut c = StatusCoalescer::new(EDGE_MS);
    let mut computations = 0usize;

    c.notice(0);
    assert!(c.begin(EDGE_MS), "the first run is due");
    computations += 1;

    // Five seconds of writes while that run is in flight.
    for tick in 0..250u64 {
        c.notice(EDGE_MS + tick * 20);
    }
    c.finish(EDGE_MS + 5_000);

    // Exactly one follow-up, whatever happened during the run.
    run_if_due(&mut c, EDGE_MS + 5_000 + EDGE_MS, &mut computations);
    assert_eq!(
        computations, 2,
        "a burst of any length is the run already going plus one that reflects it"
    );

    // And then nothing more.
    run_if_due(&mut c, 100_000, &mut computations);
    assert_eq!(
        computations, 2,
        "the follow-up must not itself schedule another"
    );
}

#[test]
fn nothing_runs_while_a_run_is_in_flight() {
    // Two overlapping status computations on a large repository is the cost this prevents; it
    // also makes the second answer arrive out of order, describing an older repository.
    let mut c = StatusCoalescer::new(EDGE_MS);
    c.notice(0);
    assert!(c.begin(EDGE_MS));
    c.notice(EDGE_MS + 1);
    assert_eq!(
        c.due(EDGE_MS + 10_000),
        Due::Idle,
        "a second run must not start"
    );
    assert!(!c.begin(EDGE_MS + 10_000));
}

#[test]
fn a_change_during_a_run_is_not_lost() {
    // The opposite failure to the one above, and just as bad: collapsing a burst so thoroughly
    // that the last change is never reflected leaves the tree permanently wrong.
    let mut c = StatusCoalescer::new(EDGE_MS);
    c.notice(0);
    assert!(c.begin(EDGE_MS));
    c.notice(EDGE_MS + 5);
    c.finish(EDGE_MS + 10);
    assert_eq!(
        c.due(EDGE_MS + 10 + EDGE_MS),
        Due::Run,
        "a change seen during a run must still produce one"
    );
}

#[test]
fn a_quiet_run_schedules_nothing() {
    let mut c = StatusCoalescer::new(EDGE_MS);
    c.notice(0);
    assert!(c.begin(EDGE_MS));
    c.finish(EDGE_MS + 10);
    assert_eq!(c.due(100_000), Due::Idle);
}

#[test]
fn the_edge_is_a_coalesce_s_number_and_not_a_new_one() {
    // Principle II. A second window would be a second thing to justify and to keep in step with
    // the one A-COALESCE already fixed.
    assert_eq!(EDGE_MS, 100);
}
