//! Connection state. Runtime only — never persisted: a connection state restored from disk
//! would be a claim about the present made from stale information.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// No report received yet; the state at launch.
    #[default]
    Unknown,
    Connecting,
    Connected,
    Disconnected,
    /// Lost, and the supervisor is waiting before another attempt (FR-020).
    ///
    /// Carries progress because "reconnecting" with no sense of when is indistinguishable
    /// from a hang — which is the complaint F000's hidden window taught this project to take
    /// seriously.
    ///
    /// `next_in_secs` rather than data-model.md's absolute `next_at`: an `Instant` has no
    /// meaning once serialised across the boundary to the webview, and what the interface
    /// actually renders is "trying again in N seconds".
    Retrying {
        attempt: u32,
        next_in_secs: u64,
    },
}

impl ConnectionState {
    /// FR-012 and SC-007: state must be distinguishable by more than colour. Every variant
    /// carries an icon name and a label so the interface cannot encode it by colour alone.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Unknown => "question",
            Self::Connecting => "circle-dashed",
            Self::Connected => "plugs-connected",
            Self::Disconnected => "plugs",
            Self::Retrying { .. } => "arrows-clockwise",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Disconnected => "Offline",
            Self::Retrying { .. } => "Reconnecting",
        }
    }

    /// FR-020: the caller can tell a transport that is still trying from one that has given
    /// up. "Given up" is `Disconnected`, reached only when the user stops it — the
    /// supervisor itself retries indefinitely.
    pub fn is_retrying(self) -> bool {
        matches!(self, Self::Retrying { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ConnectionState; 5] = [
        ConnectionState::Unknown,
        ConnectionState::Connecting,
        ConnectionState::Connected,
        ConnectionState::Disconnected,
        ConnectionState::Retrying {
            attempt: 1,
            next_in_secs: 1,
        },
    ];

    #[test]
    fn every_state_carries_a_distinct_icon_and_label() {
        let icons: Vec<_> = ALL.iter().map(|s| s.icon()).collect();
        let labels: Vec<_> = ALL.iter().map(|s| s.label()).collect();
        for set in [&icons, &labels] {
            let mut uniq = set.clone();
            uniq.sort_unstable();
            uniq.dedup();
            assert_eq!(
                uniq.len(),
                ALL.len(),
                "states must be distinguishable without colour"
            );
        }
    }

    /// T008 — the transition table. The security-relevant row is the one that is absent:
    /// nothing reaches `Retrying` from a changed host key.
    #[test]
    fn retrying_carries_its_progress() {
        let s = ConnectionState::Retrying {
            attempt: 3,
            next_in_secs: 8,
        };
        assert!(s.is_retrying());
        match s {
            ConnectionState::Retrying {
                attempt,
                next_in_secs,
            } => {
                assert_eq!(attempt, 3);
                assert_eq!(next_in_secs, 8);
            }
            _ => panic!("expected Retrying"),
        }
    }

    #[test]
    fn only_retrying_reports_itself_as_retrying() {
        for s in [
            ConnectionState::Unknown,
            ConnectionState::Connecting,
            ConnectionState::Connected,
            ConnectionState::Disconnected,
        ] {
            assert!(!s.is_retrying(), "{s:?} must not report as retrying");
        }
    }

    /// A changed host key must never reach `Retrying`. The domain expresses that through
    /// `FailureCondition::should_retry`; this asserts the two agree.
    #[test]
    fn a_changed_host_key_does_not_lead_to_retrying() {
        use crate::domain::failure::FailureCondition;
        assert!(!FailureCondition::HostKeyChanged.should_retry());
    }

    #[test]
    fn launch_state_is_unknown() {
        assert_eq!(ConnectionState::default(), ConnectionState::Unknown);
    }
}
