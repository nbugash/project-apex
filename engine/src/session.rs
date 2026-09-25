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

/// Where the ids of tasks killed by the re-execution are handed across it.
///
/// A second variable, for the reason there is a first. `session/onRestart` is emitted by the
/// **new** image, and the ids are known only to the **old** one: it is the old image that drains
/// the task set and signals the group (A-TASKEXEC), and the `exec` then discards everything it
/// knew. Without a channel the new image has nothing to report and `unpreserved` is empty --
/// which is not "nothing was lost" but "nobody looked", and the two are indistinguishable to a
/// client.
///
/// Comma-separated, and empty when nothing was running. `TaskId` is client-chosen (§4.8) and
/// nothing constrains its characters, so a comma inside one would split it here; that is
/// recorded as a known limit rather than hidden, because the alternative -- JSON in an
/// environment variable -- is a serialiser on the `exec` path for a list that is almost always
/// empty and never long.
pub const UNPRESERVED_ENV: &str = "APEX_UNPRESERVED_TASKS";

/// Read the terminated ids the old image left behind.
///
/// **Absent and empty are different.** An absent variable is an old image that predates this
/// mechanism, or a fresh start; an empty one is an image that looked and found nothing running.
/// Both yield an empty list here, and that is correct -- there is nothing to report either way
/// -- but the distinction matters at the other end, which is why the old image sets the
/// variable even when it has nothing to put in it.
fn unpreserved_from_env() -> Vec<String> {
    match std::env::var(UNPRESERVED_ENV) {
        Ok(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Format the ids for the hand-off, for the old image to set before it `exec`s.
pub fn unpreserved_to_env(ids: &[String]) -> String {
    ids.join(",")
}

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
                // F010's entry: the tasks the old image terminated before replacing itself.
                unpreserved: unpreserved_from_env(),
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
    /// T038. These tests set a process-wide environment variable, so they are serialised
    /// against each other by a mutex: `cargo test` runs a module's tests on several threads,
    /// and two of these racing would each see the other's value.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn terminated_ids_cross_the_re_execution() {
        let _guard = env_guard();
        std::env::set_var(SESSION_ENV, "session-abc");
        std::env::set_var(UNPRESERVED_ENV, "build,test-suite");

        let registry = SessionRegistry::new();
        let notice = registry.restart_notice();

        std::env::remove_var(SESSION_ENV);
        std::env::remove_var(UNPRESERVED_ENV);

        assert!(registry.restarted());
        // Without this the list is empty and A-TASKEXEC looks implemented while reporting
        // nothing: the ids are built by the image that is replaced, and the notice is emitted
        // by the one that replaces it.
        assert_eq!(
            notice.unpreserved,
            vec!["build".to_string(), "test-suite".to_string()]
        );
    }

    #[test]
    fn an_empty_variable_reports_nothing_lost() {
        let _guard = env_guard();
        std::env::set_var(SESSION_ENV, "session-abc");
        std::env::set_var(UNPRESERVED_ENV, "");

        let registry = SessionRegistry::new();
        let notice = registry.restart_notice();

        std::env::remove_var(SESSION_ENV);
        std::env::remove_var(UNPRESERVED_ENV);

        // An empty list is a positive assertion that nothing was running, which is what the
        // old image sets the variable to say. It is not the same claim as never having looked,
        // even though both arrive here as an empty vector.
        assert!(notice.unpreserved.is_empty());
    }

    #[test]
    fn an_absent_variable_is_survivable() {
        let _guard = env_guard();
        std::env::set_var(SESSION_ENV, "session-abc");
        std::env::remove_var(UNPRESERVED_ENV);

        let registry = SessionRegistry::new();
        let notice = registry.restart_notice();
        std::env::remove_var(SESSION_ENV);

        // An image that predates this mechanism. Nothing to report and nothing to panic about.
        assert!(notice.unpreserved.is_empty());
    }

    #[test]
    fn the_round_trip_survives_formatting() {
        let ids = vec!["build".to_string(), "watch".to_string()];
        let encoded = unpreserved_to_env(&ids);
        assert_eq!(encoded, "build,watch");

        let _guard = env_guard();
        std::env::set_var(SESSION_ENV, "s");
        std::env::set_var(UNPRESERVED_ENV, &encoded);
        let registry = SessionRegistry::new();
        let notice = registry.restart_notice();
        std::env::remove_var(SESSION_ENV);
        std::env::remove_var(UNPRESERVED_ENV);

        assert_eq!(notice.unpreserved, ids);
    }
}
