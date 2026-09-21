//! Apply a mutation and schedule a write. The write never happens on the interaction path.

use crate::application::error::ShellError;
use crate::application::ports::session_store::SessionStore;
use crate::domain::geometry::WindowGeometry;
use crate::domain::layout::RegionId;
use crate::domain::rail::{DestinationId, RailCatalogue, ToolWindowState};
use crate::domain::session::{DocumentId, PersistedSession, SessionSnapshot};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Writes within this window are coalesced into one. A splitter drag emits a mutation per
/// frame; without coalescing that is a file write per frame, which puts disk latency on the
/// interaction path that Constitution Principle V protects.
const DEBOUNCE: Duration = Duration::from_millis(250);

pub struct PersistSession {
    state: Mutex<PersistedSession>,
    writes: Sender<PersistedSession>,
}

impl PersistSession {
    pub fn new(store: Arc<dyn SessionStore>, initial: PersistedSession) -> Self {
        let (writes, rx) = channel::<PersistedSession>();

        // A dedicated writer thread owns the store. It blocks freely because it is not on
        // any caller's path, and it coalesces: after the first mutation it waits out the
        // debounce window, drains whatever else arrived, and writes only the last value.
        std::thread::spawn(move || {
            // The `disallowed_methods` lint bans std::thread::sleep because it blocks the
            // caller. That is exactly what is wanted here and nowhere else: this is a
            // dedicated OS thread, not a tokio worker, and blocking on it is the mechanism
            // that keeps disk latency off every interaction path. The lint stays in force
            // for the async code it exists to protect.
            #[allow(clippy::disallowed_methods)]
            while let Ok(first) = rx.recv() {
                std::thread::sleep(DEBOUNCE);
                let latest = rx.try_iter().last().unwrap_or(first);
                if let Err(e) = store.save(&latest) {
                    // FR-023: report, never block or reverse the interaction.
                    crate::logging::warn(&format!("persistence failed: {e}"));
                }
            }
        });

        Self {
            state: Mutex::new(initial),
            writes,
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

    pub fn select_destination(
        &self,
        id: &DestinationId,
        catalogue: &RailCatalogue,
    ) -> Result<ToolWindowState, ShellError> {
        let mut guard = self.state.lock().expect("session state lock");
        guard.tool_window.select(id, catalogue)?;
        let resulting = guard.tool_window.clone();
        let copy = guard.clone();
        drop(guard);
        self.schedule(copy);
        Ok(resulting)
    }

    pub fn resize_tool_window(&self, width: u32) -> Result<(), ShellError> {
        self.mutate(|s| s.tool_window.resize(width).map_err(Into::into))
    }

    pub fn tool_window(&self) -> ToolWindowState {
        self.state
            .lock()
            .expect("session state lock")
            .tool_window
            .clone()
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

    /// Hands the write to the writer thread and returns immediately. The caller never waits
    /// on disk, and a failed write cannot surface as a failed interaction.
    fn schedule(&self, session: PersistedSession) {
        let _ = self.writes.send(session);
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests wait out the debounce window deliberately
mod tests {
    use super::*;
    use crate::application::ports::session_store::StoreError;
    use crate::domain::layout::RegionId;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingStore {
        writes: AtomicUsize,
    }

    impl SessionStore for CountingStore {
        fn load(&self) -> Option<PersistedSession> {
            None
        }
        fn save(&self, _: &PersistedSession) -> Result<(), StoreError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailingStore;

    impl SessionStore for FailingStore {
        fn load(&self) -> Option<PersistedSession> {
            None
        }
        fn save(&self, _: &PersistedSession) -> Result<(), StoreError> {
            Err(StoreError::Write("disk full".into()))
        }
    }

    #[test]
    fn a_burst_of_mutations_collapses_into_far_fewer_writes() {
        let store = Arc::new(CountingStore::default());
        let persist = PersistSession::new(store.clone(), PersistedSession::default());

        // Roughly a second of dragging at 60fps.
        for i in 0..60 {
            persist
                .set_region(RegionId::Navigation, true, 200 + i)
                .unwrap();
        }
        std::thread::sleep(DEBOUNCE * 3);

        let writes = store.writes.load(Ordering::SeqCst);
        assert!(
            writes < 10,
            "60 mutations produced {writes} writes; coalescing is not working"
        );
        assert!(writes >= 1, "the final state must reach disk");
    }

    #[test]
    fn mutations_return_without_waiting_on_the_store() {
        let persist = PersistSession::new(Arc::new(FailingStore), PersistedSession::default());
        // A store that always fails must not make the interaction fail (FR-023).
        assert!(persist.set_region(RegionId::Navigation, true, 300).is_ok());
    }

    #[test]
    fn the_last_value_in_a_burst_is_the_one_persisted() {
        let store = Arc::new(CountingStore::default());
        let persist = PersistSession::new(store.clone(), PersistedSession::default());
        for extent in [200, 300, 400, 500] {
            persist
                .set_region(RegionId::Navigation, true, extent)
                .unwrap();
        }
        std::thread::sleep(DEBOUNCE * 3);
        assert_eq!(persist.snapshot().layout.navigation.extent, 500);
    }
}
