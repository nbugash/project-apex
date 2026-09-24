//! Outbound adapter: the real clock.
//!
//! Deliberately **not** behind `#[cfg(target_os = "linux")]`. It lived inside
//! `inotify_watcher.rs`, which is Linux-only, so every other platform had no `impl Clock` at
//! all -- an absence nothing noticed because nothing else needed one. A clock has nothing to do
//! with inotify, and it should not sit in the file `inotify_confinement.rs` guards.

use std::sync::OnceLock;
use std::time::Instant;

use crate::application::ports::clock::{Clock, Millis};

/// Monotonic, as the port says.
///
/// This read `SystemTime::now().duration_since(UNIX_EPOCH)` until F010 -- the wall clock, which
/// the operating system may step backwards whenever it synchronises. A coalescing window that
/// ends early because the clock jumped flushes a batch slightly too soon and nobody notices; a
/// `SIGKILL` deadline moved further away by the same step leaves a process running that was
/// meant to be killed.
#[derive(Debug, Default)]
pub struct SystemClock;

fn base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

impl Clock for SystemClock {
    fn now(&self) -> Millis {
        base().elapsed().as_millis() as Millis
    }

    fn sleep_until(&self, deadline: Millis) {
        // Re-checked in a loop: `sleep` may return early on a signal, and a caller that woke
        // before its deadline and acted on it would send a `SIGKILL` before the grace period
        // the developer was promised.
        loop {
            let now = self.now();
            if now >= deadline {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(deadline - now));
        }
    }
}
