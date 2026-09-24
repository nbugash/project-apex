//! 256 distinct paths in a rolling second becomes one invalidation (FR-015, SC-004, A-COALESCE).

use apex_engine::application::coalescer::{Coalescer, Emission, BULK_THRESHOLD};
use apex_engine::domain::watch::{RawEvent, RawKind};

fn raw(path: &str) -> RawEvent {
    RawEvent {
        kind: RawKind::Modified,
        relative_path: path.into(),
        is_directory: false,
        size: 1,
        modified: 0,
    }
}

fn after(distinct: usize) -> Emission {
    let mut c = Coalescer::default();
    for i in 0..distinct {
        c.accept(raw(&format!("f{i}.rs")), 0);
    }
    c.drain_due(200)
}

#[test]
fn one_under_the_limit_is_delivered_as_individual_events() {
    let Emission::Batch(events) = after(BULK_THRESHOLD - 1) else {
        panic!("255 paths is not wholesale");
    };
    assert_eq!(events.len(), BULK_THRESHOLD - 1);
}

#[test]
fn at_the_limit_it_is_one_invalidation_and_zero_events() {
    // "Exactly one is sent per burst, and the individual events are discarded." Delivering both
    // would be the flood the rule exists to prevent, with an invalidation on top of it.
    assert_eq!(after(BULK_THRESHOLD), Emission::InvalidateAll);
}

#[test]
fn a_branch_switch_sized_burst_is_one_invalidation() {
    assert_eq!(after(9_000), Emission::InvalidateAll);
}

#[test]
fn the_same_path_a_thousand_times_is_not_a_bulk_change() {
    // The threshold counts **distinct** paths. A compiler rewriting one output file is a
    // coalescing problem, not a wholesale one, and treating it as wholesale would throw away
    // the tree for a single file.
    let mut c = Coalescer::default();
    for tick in 0..1_000u64 {
        c.accept(raw("out/app.wasm"), tick % 50);
    }
    let Emission::Batch(events) = c.drain_due(200) else {
        panic!("one path is never wholesale");
    };
    assert_eq!(events.len(), 1);
}

#[test]
fn the_window_rolls_so_a_slow_trickle_never_trips_it() {
    // Paths spread across more than the bulk window are ordinary work, however many there are.
    let mut c = Coalescer::default();
    let mut delivered = 0usize;
    for i in 0..1_000u64 {
        // One path every 10 ms: a hundred per second, comfortably under the threshold.
        c.accept(raw(&format!("f{i}.rs")), i * 10);
        if let Emission::Batch(events) = c.drain_due(i * 10) {
            delivered += events.len();
        } else {
            panic!("a trickle must never become an invalidation");
        }
    }
    assert!(delivered > 900, "the trickle was delivered: {delivered}");
}
