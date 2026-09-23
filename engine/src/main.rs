//! `ide-engine` — the remote half of the product.
//!
//! F002 builds the first real one. It answers the handshake, owns session identity and reports
//! its own restarts, and implements no workspace method: F003 adds those. Until this binary
//! existed the far end of every connection was a mock that is forbidden to implement any §4.8
//! method, which is why `auth/handshake` had no possible responder.

fn main() {
    // Phases 2 and 4 replace this with the stdio frame loop.
    eprintln!("ide-engine {}", env!("CARGO_PKG_VERSION"));
}
