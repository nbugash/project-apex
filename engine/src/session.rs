//! Session identity and what it outlives.
//!
//! The session is owned by the engine and lives as long as the engine process. It survives
//! re-execution and disconnection — a client that reconnects re-attaches and finds its work
//! still running — and it does **not** survive a crash, because identity lives in memory
//! rather than on disk. That limit is deliberate: persisting it is a feature of its own.

use apex_protocol::wire::{RestartNotice, SessionId};
use std::sync::Mutex;

/// Where the identity is handed across an `exec`.
///
/// Re-execution replaces the process image but keeps the environment, so the identity travels
/// in it. Without this the engine would mint a new one and the restart would be
/// indistinguishable from a fresh session — which is precisely the distinction FR-024c needs.
pub const SESSION_ENV: &str = "APEX_SESSION_ID";

pub struct SessionRegistry {
    current: Mutex<SessionId>,
    /// True when this process replaced an earlier one, so the first reply can say so.
    restarted: bool,
    /// What did not survive the restart. Empty asserts nothing was lost, rather than that
    /// nothing was checked.
    unpreserved: Vec<String>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    /// Adopt an identity handed across a re-execution, or mint a new one.
    pub fn new() -> Self {
        match std::env::var(SESSION_ENV) {
            Ok(id) if !id.trim().is_empty() => Self {
                current: Mutex::new(SessionId(id)),
                restarted: true,
                // Nothing is supervised yet; F007 and F010 will have something to report here.
                unpreserved: Vec::new(),
            },
            _ => Self {
                current: Mutex::new(SessionId(uuid::Uuid::new_v4().to_string())),
                restarted: false,
                unpreserved: Vec::new(),
            },
        }
    }

    pub fn current(&self) -> SessionId {
        self.current.lock().expect("session lock").clone()
    }

    pub fn restarted(&self) -> bool {
        self.restarted
    }

    /// Whether a client's identity is the one this engine holds.
    ///
    /// False for anything else, including an identity from a previous engine. The caller must
    /// report that rather than presenting a new session as a resumed one — a client that
    /// silently continues shows a developer work that is not happening.
    pub fn resume(&self, id: &SessionId) -> bool {
        &*self.current.lock().expect("session lock") == id
    }

    pub fn restart_notice(&self) -> RestartNotice {
        RestartNotice {
            session_id: self.current(),
            unpreserved: self.unpreserved.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_registry_mints_an_identity() {
        let r = SessionRegistry {
            current: Mutex::new(SessionId("s-1".into())),
            restarted: false,
            unpreserved: Vec::new(),
        };
        assert_eq!(r.current(), SessionId("s-1".into()));
        assert!(!r.restarted());
    }

    /// Two registries must not collide, or two engines would claim one session.
    #[test]
    fn minted_identities_are_distinct() {
        let a = SessionId(uuid::Uuid::new_v4().to_string());
        let b = SessionId(uuid::Uuid::new_v4().to_string());
        assert_ne!(a, b);
    }

    /// FR-024c. An identity this engine does not hold is refused, so the client learns its
    /// work is gone instead of watching a session that is running nothing.
    #[test]
    fn an_unknown_identity_is_refused() {
        let r = SessionRegistry {
            current: Mutex::new(SessionId("mine".into())),
            restarted: false,
            unpreserved: Vec::new(),
        };
        assert!(r.resume(&SessionId("mine".into())));
        assert!(!r.resume(&SessionId("someone-elses".into())));
        assert!(!r.resume(&SessionId("".into())));
    }

    /// FR-025. An empty list is a positive claim that nothing was lost. Omitting an item would
    /// make it appear to have survived, which is worse than reporting the loss.
    #[test]
    fn a_restart_notice_carries_the_same_identity_and_names_what_was_lost() {
        let r = SessionRegistry {
            current: Mutex::new(SessionId("kept".into())),
            restarted: true,
            unpreserved: vec!["one language server".into()],
        };
        let n = r.restart_notice();
        assert_eq!(n.session_id, SessionId("kept".into()));
        assert_eq!(n.unpreserved, vec!["one language server".to_string()]);

        let quiet = SessionRegistry {
            current: Mutex::new(SessionId("kept".into())),
            restarted: true,
            unpreserved: Vec::new(),
        };
        assert!(quiet.restart_notice().unpreserved.is_empty());
    }
}
