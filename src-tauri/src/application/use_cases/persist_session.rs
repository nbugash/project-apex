//! Apply a mutation and schedule a write. The write never happens on the interaction path.

use crate::application::error::ShellError;
use crate::application::ports::session_store::SessionStore;
use crate::domain::geometry::WindowGeometry;
use crate::domain::layout::RegionId;
use crate::domain::session::{DocumentId, PersistedSession, SessionSnapshot};
use std::sync::{Arc, Mutex};

pub struct PersistSession {
    store: Arc<dyn SessionStore>,
    state: Mutex<PersistedSession>,
}

impl PersistSession {
    pub fn new(store: Arc<dyn SessionStore>, initial: PersistedSession) -> Self {
        Self {
            store,
            state: Mutex::new(initial),
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot::from(&*self.state.lock().expect("session state lock"))
    }

    pub fn set_region(&self, id: RegionId, visible: bool, extent: u32) -> Result<(), ShellError> {
        self.mutate(|s| s.layout.set_region(id, visible, extent).map_err(Into::into))
    }

    pub fn open_document(&self, display_name: &str) -> Result<DocumentId, ShellError> {
        let mut guard = self.state.lock().expect("session state lock");
        let id = guard.open_document(display_name)?;
        let copy = guard.clone();
        drop(guard);
        self.schedule(copy);
        Ok(id)
    }

    pub fn close_document(&self, id: &DocumentId) -> Result<(), ShellError> {
        self.mutate(|s| s.close_document(id).map_err(Into::into))
    }

    pub fn reorder_document(&self, id: &DocumentId, to: u32) -> Result<(), ShellError> {
        self.mutate(|s| s.reorder_document(id, to).map_err(Into::into))
    }

    pub fn focus_document(&self, id: &DocumentId) -> Result<(), ShellError> {
        self.mutate(|s| s.focus_document(id).map_err(Into::into))
    }

    /// Geometry is observed from native window events, so it cannot fail and is not a
    /// command (see contracts/shell-commands.md, Non-goals).
    pub fn record_geometry(&self, geometry: WindowGeometry) {
        let _ = self.mutate(|s| {
            s.window = geometry;
            Ok(())
        });
    }

    fn mutate<F>(&self, f: F) -> Result<(), ShellError>
    where
        F: FnOnce(&mut PersistedSession) -> Result<(), ShellError>,
    {
        let mut guard = self.state.lock().expect("session state lock");
        f(&mut guard)?;
        let copy = guard.clone();
        drop(guard);
        self.schedule(copy);
        Ok(())
    }

    /// Debounced in the adapter that owns the timer; here the write is simply off the
    /// caller's critical path and its failure is not propagated to the caller.
    fn schedule(&self, session: PersistedSession) {
        if let Err(e) = self.store.save(&session) {
            // FR-023: report, never block or reverse the interaction.
            eprintln_log(&format!("persistence failed: {e}"));
        }
    }
}

fn eprintln_log(msg: &str) {
    crate::logging::warn(msg);
}
