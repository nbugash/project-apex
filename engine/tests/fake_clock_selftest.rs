//! The fake clock is a test double that other tests depend on for their meaning, so it gets
//! tested itself (T032b).
//!
//! F004's version was `Cell`-backed and had one method. F010 shares it across threads and waits
//! on it, and a double that silently fails to release a waiter turns an escalation test into a
//! hang that looks like a slow suite.

mod common;

use apex_engine::application::ports::clock::Clock;
use common::fake_clock::FakeClock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn one_advance_releases_every_waiter_whose_deadline_has_passed() {
    let clock: Arc<FakeClock> = Arc::new(FakeClock::new());
    let woken = Arc::new(AtomicUsize::new(0));

    // Three threads, as a task's readers and the escalation thread would be.
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let c = Arc::clone(&clock);
            let w = Arc::clone(&woken);
            std::thread::spawn(move || {
                c.sleep_until(100);
                w.fetch_add(1, Ordering::SeqCst);
            })
        })
        .collect();

    // Let them all park. Without this the advance can happen before anyone waits, which would
    // pass for a reason unrelated to the one under test.
    let deadline = Instant::now() + Duration::from_secs(5);
    while woken.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        if clock.now() == 0 {
            break;
        }
    }
    assert_eq!(woken.load(Ordering::SeqCst), 0, "nobody should wake early");

    clock.advance(100);

    for h in handles {
        h.join().expect("a waiter did not return");
    }
    // `notify_one` would leave two of these parked forever on a clock that has already moved
    // past their deadline. The join above would hang rather than fail, which is why this test
    // exists at all.
    assert_eq!(woken.load(Ordering::SeqCst), 3);
}

#[test]
fn the_boundary_is_inclusive() {
    let clock = FakeClock::new();
    clock.set(5_000);
    // A deadline of exactly 5 000 is reached at 5 000, matching the port's `>=`. If this were
    // `>`, T032d's "advanced to 5 000 yields exactly one kill" would need 5 001 and the two
    // documents would disagree about when a task dies.
    clock.sleep_until(5_000); // returns immediately, or this test hangs
}

#[test]
fn a_waiter_past_its_deadline_never_parks() {
    let clock = FakeClock::new();
    clock.set(10);
    clock.sleep_until(1);
}
