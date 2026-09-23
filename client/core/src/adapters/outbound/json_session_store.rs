//! `SessionStore` over a JSON file in the application-data directory.
//!
//! A-STATE: interface session state lives outside the workspace cache. The payload has none
//! of the properties that justify a database, and the two have different lifetimes — a cache
//! eviction must not destroy a user's layout.

use crate::application::ports::session_store::{SessionStore, StoreError};
use crate::domain::session::PersistedSession;
use crate::logging;
use std::fs;
use std::path::{Path, PathBuf};

pub struct JsonFileSessionStore {
    path: PathBuf,
}

impl JsonFileSessionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SessionStore for JsonFileSessionStore {
    fn load(&self) -> Option<PersistedSession> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                logging::warn(&format!("session state unreadable, using defaults: {e}"));
                return None;
            }
        };
        match serde_json::from_str::<PersistedSession>(&raw) {
            Ok(s) => Some(s),
            Err(e) => {
                // Discarded in full rather than partially recovered: a half-restored layout
                // is harder to reason about than a default one. Overwritten on next save.
                logging::warn(&format!("session state malformed, using defaults: {e}"));
                None
            }
        }
    }

    fn save(&self, session: &PersistedSession) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| StoreError::Write(e.to_string()))?;
        }
        let body =
            serde_json::to_string_pretty(session).map_err(|e| StoreError::Write(e.to_string()))?;
        // Write-then-rename so a crash mid-write cannot leave a partially written file that
        // the next launch would read as corrupt.
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, body).map_err(|e| StoreError::Write(e.to_string()))?;
        fs::rename(&tmp, &self.path).map_err(|e| StoreError::Write(e.to_string()))?;
        Ok(())
    }
}
