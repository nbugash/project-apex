#![allow(clippy::disallowed_methods)] // a test thread polling a flag has no runtime to starve

//! Integration: no background work outlives the session that started it (FR-019, SC-010).
//!
//! The shell owns exactly one background thread — the persistence writer. If it kept running
//! after the session was dropped it would hold the store open and the process would not
//! exit cleanly. This proves it releases its reference and ends.

use apex_shell::application::ports::session_store::{SessionStore, StoreError};
use apex_shell::application::use_cases::persist_session::PersistSession;
use apex_shell::domain::layout::RegionId;
use apex_shell::domain::session::PersistedSession;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Sets a flag when dropped. The writer thread holds the only other reference, so the store
/// is dropped exactly when that thread ends.
struct DropTrackingStore {
    dropped: Arc<AtomicBool>,
    writes: Arc<AtomicBool>,
}

impl SessionStore for DropTrackingStore {
    fn load(&self) -> Option<PersistedSession> {
        None
    }
    fn save(&self, _: &PersistedSession) -> Result<(), StoreError> {
        self.writes.store(true, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for DropTrackingStore {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn wait_for(flag: &AtomicBool, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn the_persistence_writer_ends_when_the_session_is_dropped() {
    let dropped = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicBool::new(false));
    let store = Arc::new(DropTrackingStore {
        dropped: dropped.clone(),
        writes: writes.clone(),
    });

    let persist = PersistSession::new(store, PersistedSession::default());
    persist.set_region(RegionId::Output, true, 300).unwrap();

    assert!(
        !dropped.load(Ordering::SeqCst),
        "the writer must still be running while the session is alive"
    );

    drop(persist);

    assert!(
        wait_for(&dropped, Duration::from_secs(5)),
        "the writer thread outlived the session: nothing must survive quit (FR-019)"
    );
}

#[test]
fn a_pending_write_is_not_lost_when_the_session_is_dropped() {
    let dropped = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicBool::new(false));
    let store = Arc::new(DropTrackingStore {
        dropped: dropped.clone(),
        writes: writes.clone(),
    });

    let persist = PersistSession::new(store, PersistedSession::default());
    persist.set_region(RegionId::Output, true, 250).unwrap();
    drop(persist);

    // Shutting down must not silently discard the user's last change.
    assert!(
        wait_for(&writes, Duration::from_secs(5)),
        "the queued write was dropped on shutdown"
    );
}
