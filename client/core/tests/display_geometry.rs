//! Integration: restored geometry is always constrained to an attached display (FR-009,
//! SC-009). Exercised through the restore use case rather than the domain alone, so the
//! store, the coherence check and the repair path are all in the loop.

use apex_shell::adapters::outbound::json_session_store::JsonFileSessionStore;
use apex_shell::application::ports::session_store::SessionStore;
use apex_shell::application::use_cases::restore_session::RestoreSession;
use apex_shell::domain::geometry::{
    DisplayBounds, WindowGeometry, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
};
use apex_shell::domain::session::PersistedSession;
use std::sync::Arc;

fn laptop() -> Vec<DisplayBounds> {
    vec![DisplayBounds {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }]
}

fn restore_with(geometry: WindowGeometry, displays: &[DisplayBounds]) -> WindowGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let store = JsonFileSessionStore::new(path.clone());
    let session = PersistedSession {
        window: geometry,
        ..PersistedSession::default()
    };
    store.save(&session).unwrap();

    RestoreSession::new(Arc::new(JsonFileSessionStore::new(path)))
        .execute(displays)
        .window
}

#[test]
fn geometry_saved_on_a_still_attached_display_is_restored_verbatim() {
    let g = WindowGeometry {
        x: 300,
        y: 200,
        width: 1100,
        height: 750,
        maximized: false,
    };
    assert_eq!(restore_with(g, &laptop()), g);
}

#[test]
fn the_undocked_laptop_case_recovers_onto_the_remaining_display() {
    // Saved while docked to an external monitor to the right, which is now detached.
    let g = WindowGeometry {
        x: 2400,
        y: 300,
        width: 1100,
        height: 750,
        maximized: false,
    };
    let out = restore_with(g, &laptop());
    assert!(
        out.title_bar_reachable(&laptop()),
        "window must be reachable, not merely on-screen"
    );
}

#[test]
fn geometry_smaller_than_the_minimum_is_enlarged() {
    let g = WindowGeometry {
        x: 10,
        y: 10,
        width: 320,
        height: 240,
        maximized: false,
    };
    let out = restore_with(g, &laptop());
    assert!(out.width >= MIN_WINDOW_WIDTH && out.height >= MIN_WINDOW_HEIGHT);
}

#[test]
fn a_maximized_window_keeps_its_flag_through_restore() {
    let g = WindowGeometry {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
        maximized: true,
    };
    assert!(restore_with(g, &laptop()).maximized);
}
