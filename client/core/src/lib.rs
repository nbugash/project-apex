//! Apex Shell core.
//!
//! Layered per Constitution Principle VIII: `domain` depends on nothing external,
//! `application` depends only on ports, and `adapters` depend inward. Framework types stay
//! in adapters — nothing in `domain` or `application` imports Tauri.

pub mod adapters;
pub mod application;
pub mod composition;
pub mod composition_workspace;
pub mod domain;
pub mod logging;
pub mod window;

use adapters::inbound::tauri_commands as cmd;
use std::sync::Arc;
use tauri::{Emitter, Manager};

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            cmd::shell_ready,
            cmd::session_get,
            cmd::layout_set_region,
            cmd::documents_open,
            cmd::documents_close,
            cmd::documents_reorder,
            cmd::documents_focus,
            cmd::connection_current,
            cmd::rail_select,
            cmd::tool_window_resize,
            cmd::rail_destinations,
            cmd::workspace_read_directory,
            #[cfg(debug_assertions)]
            cmd::stub_set_connection,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            // APEX_DATA_DIR lets the end-to-end suite seed, corrupt and isolate session
            // state per test. Without it every test would share one real profile directory
            // and could only run in a fixed order, which is how flaky suites start.
            let data_dir = std::env::var_os("APEX_DATA_DIR")
                .map(std::path::PathBuf::from)
                .or_else(|| handle.path().app_data_dir().ok())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            logging::init(data_dir.join("shell.log"));
            logging::info("shell starting");

            let window = Arc::new(window::controller::WindowController::from_app(&handle)?);
            let wiring = composition::build(data_dir, window.clone());

            // Connection transitions reach the interface as events, emitted once at startup
            // so the interface never polls for an initial value.
            let emitter = handle.clone();
            wiring.shell.connection.start(Box::new(move |state| {
                let _ = emitter.emit("connection:changed", state);
            }));

            // The same contract for the workspace: emitted once at startup with the
            // restored value, so the interface never has to ask for an initial state.
            let _ = handle.emit(
                "workspace:changed",
                wiring.shell.persist.snapshot().workspace,
            );

            // Geometry is observed from native events, never reported by the interface:
            // the webview does not own positions it cannot authoritatively know.
            let persist = wiring.shell.persist.clone();
            let geometry_window = window.clone();
            let main = handle
                .get_webview_window("main")
                .expect("main window exists after controller construction");
            main.on_window_event(move |event| {
                if matches!(
                    event,
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                ) {
                    if let Some(g) = geometry_window.current_geometry() {
                        persist.record_geometry(g);
                    }
                }
            });

            app.manage(wiring.shell);
            app.manage(wiring.workspace);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Apex Shell")
        .run(|_app, event| {
            // FR-019: nothing outlives the window. The persistence writer thread holds the
            // only other handle, and dropping the sender ends it; logging this makes an
            // unclean exit visible rather than silent.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                logging::info("shell shutting down");
            }
        });
}
