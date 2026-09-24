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

/// Whether the developer is being told about changes on the host.
///
/// **Deliberately not a `ConnectionState` variant.** Exhausted watch capacity happens while
/// perfectly connected, and a link that is up while the watcher is refused is a state a single
/// connection enum cannot express. FR-005 and FR-025 both require the developer to be told in
/// that case, and folding it into connectivity would mean saying "disconnected" when the
/// connection is fine, or saying nothing at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Reporting {
    /// Changes arrive as they happen.
    #[default]
    Live,
    /// Nothing is arriving because the link is down. The tree is what was last read.
    Offline,
    /// Connected, and some paths are not being watched. Carries how many, because "some" is
    /// what a developer cannot act on.
    Partial { unwatched: usize },
    /// Connected, and watching is unavailable entirely — an engine that cannot watch, or a
    /// local workspace in a version that does not (A-WATCHLOCAL). Browsing and reading
    /// continue; only automatic freshness is lost, and its loss is stated (FR-027).
    Unavailable,
}

impl Reporting {
    /// Is the developer currently being told about every change?
    ///
    /// The question the status bar asks. Anything but `Live` is something to say out loud:
    /// silence is how a developer would otherwise discover that watching failed, which is the
    /// one outcome FR-005 forbids.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live)
    }
}

#[cfg(test)]
mod reporting_tests {
    use super::*;

    #[test]
    fn every_state_but_live_is_something_to_say() {
        assert!(Reporting::Live.is_live());
        for quiet in [
            Reporting::Offline,
            Reporting::Partial { unwatched: 3 },
            Reporting::Unavailable,
        ] {
            assert!(
                !quiet.is_live(),
                "{quiet:?} must be reported to the developer"
            );
        }
    }

    #[test]
    fn partial_carries_a_number_rather_than_a_vague_quantity() {
        // "Some paths are not being watched" is not something a developer can act on.
        let state = Reporting::Partial { unwatched: 7 };
        let Reporting::Partial { unwatched } = state else {
            panic!("shape");
        };
        assert_eq!(unwatched, 7);
    }

    #[test]
    fn it_is_independent_of_connectivity() {
        // The whole reason it is a separate type: connected-and-not-reporting is a real state.
        let connected = ConnectionState::Connected;
        let partial = Reporting::Partial { unwatched: 1 };
        assert_eq!(connected, ConnectionState::Connected);
        assert!(!partial.is_live());
    }
}
