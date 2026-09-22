//! Why a connection or an attempt failed, and what follows from it.
//!
//! The six conditions §3.4 enumerates, plus `Unknown` for anything matching none of them.
//! `Unknown` exists so an unclassifiable failure is visible rather than quietly mapped onto
//! a neighbour — a wrong classification is worse than an honest "I do not know", because it
//! sends the user down a remedy that cannot work.

use serde::{Deserialize, Serialize};

/// How much stderr is retained for classification.
///
/// Bounded because §3.4 makes the engine's stderr discipline normative but cannot enforce
/// it, and neither can this code: no framing on stdout constrains stderr. A bound means a
/// noisy engine degrades classification instead of exhausting memory.
pub const MAX_STDERR_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCondition {
    /// exit 255 with a timeout or refusal. Retry.
    HostUnreachable,
    /// exit 255 with permission denied. The only condition that may raise a prompt.
    AuthenticationFailed,
    /// The host's identity changed. **Never retried** — see `should_retry`.
    HostKeyChanged,
    /// exit 127. The engine is not installed; F002 owns putting it there.
    EngineMissing,
    /// Non-zero but not 255. The engine started and died.
    EngineCrashed,
    /// The pipe ended, or OpenSSH's keepalive gave up and `ssh` exited.
    NetworkDropped,
    /// Matched nothing. Surfaced verbatim, never retried.
    Unknown,
}

impl FailureCondition {
    /// Whether the supervisor may try again.
    ///
    /// `HostKeyChanged` is false and must stay false. Retrying a possible
    /// machine-in-the-middle is worse than failing, and an IDE that keeps trying trains its
    /// users to dismiss the warning — which is the entire protection.
    ///
    /// `EngineMissing` is false because retrying cannot install anything; it is handed to
    /// F002. `Unknown` is false because retrying a failure nobody understands is guessing.
    pub fn should_retry(self) -> bool {
        match self {
            Self::HostUnreachable | Self::EngineCrashed | Self::NetworkDropped => true,
            Self::AuthenticationFailed
            | Self::HostKeyChanged
            | Self::EngineMissing
            | Self::Unknown => false,
        }
    }

    /// Whether this condition may raise a credential prompt (FR-006).
    ///
    /// Only one may. Prompting because a host is unreachable teaches the user their
    /// credential is wrong when the problem is the network.
    pub fn may_prompt_for_credential(self) -> bool {
        matches!(self, Self::AuthenticationFailed)
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::HostUnreachable => "the host could not be reached",
            Self::AuthenticationFailed => "the host refused the credential",
            Self::HostKeyChanged => "the host's identity has changed",
            Self::EngineMissing => "the engine is not installed on the host",
            Self::EngineCrashed => "the engine started and then exited",
            Self::NetworkDropped => "the connection to the host was lost",
            Self::Unknown => "the connection failed for an unrecognised reason",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The security-critical row. If this test is ever changed to pass with `true`, the
    /// change is the defect.
    #[test]
    fn a_changed_host_key_is_never_retried() {
        assert!(!FailureCondition::HostKeyChanged.should_retry());
    }

    #[test]
    fn only_authentication_failure_may_prompt() {
        for c in [
            FailureCondition::HostUnreachable,
            FailureCondition::HostKeyChanged,
            FailureCondition::EngineMissing,
            FailureCondition::EngineCrashed,
            FailureCondition::NetworkDropped,
            FailureCondition::Unknown,
        ] {
            assert!(!c.may_prompt_for_credential(), "{c:?} must not prompt");
        }
        assert!(FailureCondition::AuthenticationFailed.may_prompt_for_credential());
    }

    #[test]
    fn a_missing_engine_is_handed_off_not_retried() {
        // Retrying cannot install anything. F002 owns the bootstrap.
        assert!(!FailureCondition::EngineMissing.should_retry());
    }

    #[test]
    fn an_unrecognised_failure_is_not_retried() {
        assert!(!FailureCondition::Unknown.should_retry());
    }

    #[test]
    fn every_condition_describes_itself() {
        for c in [
            FailureCondition::HostUnreachable,
            FailureCondition::AuthenticationFailed,
            FailureCondition::HostKeyChanged,
            FailureCondition::EngineMissing,
            FailureCondition::EngineCrashed,
            FailureCondition::NetworkDropped,
            FailureCondition::Unknown,
        ] {
            assert!(!c.describe().is_empty());
        }
    }
}
