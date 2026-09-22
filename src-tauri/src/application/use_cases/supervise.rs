//! Connection supervision: when to try again, and how long to wait.
//!
//! The policy lives here, apart from the IO that enacts it, because a retry schedule is the
//! kind of thing that is easy to get subtly wrong and hard to observe once it is tangled
//! with process handling. A tight retry loop passes a "did it reconnect" test and is still
//! wrong.

use crate::domain::failure::FailureCondition;
use std::time::Duration;

/// First wait after a loss.
pub const BASE_DELAY: Duration = Duration::from_secs(1);
/// Ceiling. A laptop shut for an hour should reconnect within half a minute of waking, not
/// back off to hours — which is what uncapped doubling produces.
pub const MAX_DELAY: Duration = Duration::from_secs(30);

/// Exponential backoff with full jitter.
///
/// Jitter rather than plain doubling because every client of a restarted bastion would
/// otherwise retry in lockstep, and the thundering herd is what keeps it down.
#[derive(Debug, Clone)]
pub struct Backoff {
    attempt: u32,
    base: Duration,
    max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(BASE_DELAY, MAX_DELAY)
    }
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            attempt: 0,
            base,
            max,
        }
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The ceiling for the next wait, before jitter. Exposed so a test can assert growth
    /// without depending on a random draw.
    pub fn next_ceiling(&self) -> Duration {
        let factor = 1u64 << self.attempt.min(20);
        self.base
            .saturating_mul(factor.min(u32::MAX as u64) as u32)
            .min(self.max)
    }

    /// Advance and return how long to wait.
    ///
    /// Full jitter: a uniform draw from `[0, ceiling]` rather than `ceiling ± something`.
    /// It spreads a herd better, and the cost — an occasional very short wait — is harmless
    /// when the ceiling is what bounds the worst case.
    pub fn next_delay(&mut self, random_fraction: f64) -> Duration {
        let ceiling = self.next_ceiling();
        self.attempt = self.attempt.saturating_add(1);
        let f = random_fraction.clamp(0.0, 1.0);
        ceiling.mul_f64(f)
    }

    /// A connection came back. The next loss starts from the base delay again.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// What the supervisor does about a failure.
///
/// Separated from the classification itself so the policy is readable in one place: the
/// table in data-model.md is this function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// Wait out the backoff and try again.
    RetryAfterBackoff,
    /// Try again immediately with the assisted phase (FR-006).
    AttemptAssisted,
    /// Stop. The user must act — a changed host key, or a failure nobody understands.
    StopAndReport,
    /// Stop and hand to the feature that installs the engine (F002).
    HandToBootstrap,
}

pub fn response_to(condition: FailureCondition, assisted_available: bool) -> Response {
    match condition {
        FailureCondition::AuthenticationFailed if assisted_available => Response::AttemptAssisted,
        // Without the assisted phase — an OpenSSH too old for it — the honest next step is
        // the identity picker, which is a report to the user rather than another attempt.
        FailureCondition::AuthenticationFailed => Response::StopAndReport,
        FailureCondition::EngineMissing => Response::HandToBootstrap,
        FailureCondition::HostKeyChanged | FailureCondition::Unknown => Response::StopAndReport,
        c if c.should_retry() => Response::RetryAfterBackoff,
        _ => Response::StopAndReport,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_ceiling_grows_then_stops_at_the_maximum() {
        let mut b = Backoff::default();
        let mut ceilings = Vec::new();
        for _ in 0..10 {
            ceilings.push(b.next_ceiling());
            b.next_delay(1.0);
        }
        // Growing, not constant. A fixed interval either hammers a down host or reconnects
        // slowly, and cannot be both.
        assert!(
            ceilings.windows(2).all(|w| w[1] >= w[0]),
            "the interval must not shrink: {ceilings:?}"
        );
        assert!(ceilings[3] > ceilings[0], "the interval must actually grow");
        assert_eq!(
            *ceilings.last().unwrap(),
            MAX_DELAY,
            "and then stop growing"
        );
    }

    #[test]
    fn the_first_wait_is_the_base_delay() {
        assert_eq!(Backoff::default().next_ceiling(), BASE_DELAY);
    }

    #[test]
    fn jitter_spreads_the_wait_across_the_whole_window() {
        let mut b = Backoff::new(Duration::from_secs(8), MAX_DELAY);
        assert_eq!(b.clone().next_delay(0.0), Duration::ZERO);
        assert_eq!(b.clone().next_delay(1.0), Duration::from_secs(8));
        assert_eq!(b.next_delay(0.5), Duration::from_secs(4));
    }

    #[test]
    fn a_restored_connection_resets_the_schedule() {
        let mut b = Backoff::default();
        for _ in 0..5 {
            b.next_delay(1.0);
        }
        assert!(b.next_ceiling() > BASE_DELAY);
        b.reset();
        assert_eq!(b.next_ceiling(), BASE_DELAY);
        assert_eq!(b.attempt(), 0);
    }

    #[test]
    fn retrying_forever_does_not_overflow() {
        let mut b = Backoff::default();
        for _ in 0..1000 {
            b.next_delay(1.0);
        }
        assert_eq!(b.next_ceiling(), MAX_DELAY);
    }

    // --- the response table ---

    #[test]
    fn a_changed_host_key_stops_rather_than_retrying() {
        assert_eq!(
            response_to(FailureCondition::HostKeyChanged, true),
            Response::StopAndReport
        );
    }

    #[test]
    fn only_an_authentication_failure_reaches_the_assisted_phase() {
        assert_eq!(
            response_to(FailureCondition::AuthenticationFailed, true),
            Response::AttemptAssisted
        );
        for c in [
            FailureCondition::HostUnreachable,
            FailureCondition::NetworkDropped,
            FailureCondition::EngineCrashed,
            FailureCondition::EngineMissing,
            FailureCondition::HostKeyChanged,
            FailureCondition::Unknown,
        ] {
            assert_ne!(
                response_to(c, true),
                Response::AttemptAssisted,
                "{c:?} must not raise a credential prompt"
            );
        }
    }

    #[test]
    fn an_old_openssh_falls_back_to_reporting_rather_than_prompting() {
        assert_eq!(
            response_to(FailureCondition::AuthenticationFailed, false),
            Response::StopAndReport
        );
    }

    #[test]
    fn a_missing_engine_is_handed_to_the_bootstrap_feature() {
        assert_eq!(
            response_to(FailureCondition::EngineMissing, true),
            Response::HandToBootstrap
        );
    }

    #[test]
    fn transient_failures_retry() {
        for c in [
            FailureCondition::HostUnreachable,
            FailureCondition::NetworkDropped,
            FailureCondition::EngineCrashed,
        ] {
            assert_eq!(response_to(c, true), Response::RetryAfterBackoff, "{c:?}");
        }
    }

    #[test]
    fn an_unrecognised_failure_stops_rather_than_guessing() {
        assert_eq!(
            response_to(FailureCondition::Unknown, true),
            Response::StopAndReport
        );
    }
}
