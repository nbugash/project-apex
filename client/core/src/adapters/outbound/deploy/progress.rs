//! Deciding when to publish transfer progress.
//!
//! Separated from the transfer so the cadence rule is testable without moving bytes. FR-009
//! requires a report at least once per second, and "visible" was rejected as untestable: a
//! single message at the start satisfies it while still looking exactly like a hang.

use std::time::{Duration, Instant};

/// The longest a transfer may run without saying anything.
pub const MAX_SILENCE: Duration = Duration::from_secs(1);

pub struct ProgressTicker {
    last: Instant,
}

impl Default for ProgressTicker {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressTicker {
    pub fn new() -> Self {
        Self {
            last: Instant::now(),
        }
    }

    /// Whether to publish now. Always true on completion, so the final state is never left
    /// implied by silence.
    pub fn should_publish(&mut self, sent: u64, total: u64) -> bool {
        self.decide(sent, total, Instant::now())
    }

    /// The decision, with time injected. A cadence rule tested against the real clock is a
    /// test that sleeps, and a test that sleeps is one people delete.
    pub fn decide(&mut self, sent: u64, total: u64, now: Instant) -> bool {
        if sent >= total || now.duration_since(self.last) >= MAX_SILENCE {
            self.last = now;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_published_between_ticks() {
        let start = Instant::now();
        let mut t = ProgressTicker { last: start };
        assert!(!t.decide(10, 1000, start + Duration::from_millis(200)));
        assert!(!t.decide(20, 1000, start + Duration::from_millis(900)));
    }

    #[test]
    fn a_second_of_silence_forces_a_report() {
        let start = Instant::now();
        let mut t = ProgressTicker { last: start };
        assert!(t.decide(50, 1000, start + Duration::from_millis(1000)));
    }

    /// The property FR-009 is actually about: over a long transfer, no gap exceeds a second.
    /// Asserting "some progress was reported" would pass against one message at the start.
    #[test]
    fn no_gap_in_a_long_transfer_exceeds_a_second() {
        let start = Instant::now();
        let mut t = ProgressTicker { last: start };
        let mut last_published = start;
        // 60 seconds of transfer, sampled every 100 ms as a chunk loop would.
        for ms in (100..=60_000).step_by(100) {
            let now = start + Duration::from_millis(ms);
            if t.decide(ms, 60_001, now) {
                last_published = now;
            }
            assert!(
                now.duration_since(last_published) < MAX_SILENCE,
                "went {:?} without a report",
                now.duration_since(last_published)
            );
        }
    }

    #[test]
    fn completion_always_reports_even_if_it_arrives_early() {
        let start = Instant::now();
        let mut t = ProgressTicker { last: start };
        assert!(
            t.decide(1000, 1000, start + Duration::from_millis(5)),
            "the final state must never be left implied by silence"
        );
    }
}
