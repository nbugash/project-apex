//! The real clock.

use crate::application::ports::clock::Clock;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            // Before 1970 the machine's clock is wrong in a way this feature cannot fix. Zero
            // makes every cache entry look ancient, which evicts rather than serving something
            // whose age cannot be reasoned about.
            .unwrap_or(0)
    }
}
