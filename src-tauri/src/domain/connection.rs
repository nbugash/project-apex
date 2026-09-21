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
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Disconnected => "Offline",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ConnectionState; 4] = [
        ConnectionState::Unknown,
        ConnectionState::Connecting,
        ConnectionState::Connected,
        ConnectionState::Disconnected,
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

    #[test]
    fn launch_state_is_unknown() {
        assert_eq!(ConnectionState::default(), ConnectionState::Unknown);
    }
}
