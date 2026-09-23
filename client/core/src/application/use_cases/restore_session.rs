//! Load, validate, repair, or fall back to defaults. Never fails.

use crate::application::ports::session_store::SessionStore;
use crate::domain::geometry::DisplayBounds;
use crate::domain::session::{PersistedSession, SessionSnapshot};
use std::sync::Arc;

pub struct RestoreSession {
    store: Arc<dyn SessionStore>,
}

impl RestoreSession {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store }
    }

    /// Always returns a usable session. A file that is absent, unreadable, malformed or
    /// incoherent yields defaults, and the caller cannot tell the difference — by design.
    pub fn execute(&self, displays: &[DisplayBounds]) -> PersistedSession {
        self.store
            .load()
            .filter(PersistedSession::is_coherent)
            .unwrap_or_default()
            .repaired(displays)
    }

    pub fn snapshot(&self, displays: &[DisplayBounds]) -> SessionSnapshot {
        SessionSnapshot::from(&self.execute(displays))
    }
}
