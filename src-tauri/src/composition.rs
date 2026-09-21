//! Composition root. The only place adapters are bound to use cases.
//!
//! Constitution Principle VIII: wiring lives here, not scattered behind hidden globals or a
//! service locator. Swapping the stub connection source for the real transport in F001 is a
//! one-line change in this file.

use crate::adapters::inbound::tauri_commands::Shell;
use crate::adapters::outbound::json_session_store::JsonFileSessionStore;
use crate::adapters::outbound::stub_connection::StubConnectionStatusSource;
use crate::application::ports::connection::ConnectionStatusSource;
use crate::application::ports::session_store::SessionStore;
use crate::application::use_cases::observe_connection::ObserveConnection;
use crate::application::use_cases::persist_session::PersistSession;
use crate::application::use_cases::restore_session::RestoreSession;
use crate::window::controller::WindowController;
use std::path::PathBuf;
use std::sync::Arc;

pub struct Wiring {
    pub shell: Shell,
    pub stub: Arc<StubConnectionStatusSource>,
}

pub fn build(data_dir: PathBuf, window: Arc<WindowController>) -> Wiring {
    let store: Arc<dyn SessionStore> =
        Arc::new(JsonFileSessionStore::new(data_dir.join("session.json")));

    let restored = RestoreSession::new(store.clone()).execute(&window.attached_displays());
    if let Err(e) = window.apply(&restored.window) {
        crate::logging::warn(&format!("could not apply restored geometry: {e}"));
    }

    let persist = Arc::new(PersistSession::new(store, restored));

    let stub = Arc::new(StubConnectionStatusSource::new());
    let source: Arc<dyn ConnectionStatusSource> = stub.clone();
    let connection = Arc::new(ObserveConnection::new(source));

    Wiring {
        shell: Shell {
            persist,
            connection,
            window,
        },
        stub,
    }
}
