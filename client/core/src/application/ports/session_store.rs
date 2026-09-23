//! Outbound port: load and persist session state.

use crate::domain::session::PersistedSession;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("session state could not be written: {0}")]
    Write(String),
}

pub trait SessionStore: Send + Sync {
    /// `Ok(None)` for absent **or** unreadable **or** invalid content: invalidity is
    /// absence, not an error. Falling back to defaults is a normal path (FR-008).
    fn load(&self) -> Option<PersistedSession>;

    /// Durable, or `StoreError`. Never a partially written file.
    fn save(&self, session: &PersistedSession) -> Result<(), StoreError>;
}
