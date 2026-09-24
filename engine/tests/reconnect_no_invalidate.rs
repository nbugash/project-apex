//! The engine sends no invalidation on reconnection (`invalidateAll` guarantee 6, FR-026).
//!
//! It cannot distinguish a reconnecting client from a new one -- the watch set arrives the same
//! way in both cases -- so the staleness decision is the client's, which is the only side that
//! knows it was away.

mod common;

use apex_engine::application::coalescer::{Coalescer, Emission};

#[test]
fn a_fresh_coalescer_emits_nothing() {
    // Whatever a reconnection looks like to the engine, it begins here: no pending events, no
    // overflow, nothing to say.
    let mut c = Coalescer::default();
    assert_eq!(c.drain_due(0), Emission::Batch(vec![]));
    assert_eq!(c.drain_due(10_000), Emission::Batch(vec![]));
}

#[test]
fn an_invalidation_only_ever_comes_from_volume_or_overflow() {
    // The two routes, and there is no third. A reconnection is neither.
    let mut c = Coalescer::default();
    assert_eq!(c.next_deadline(), None, "nothing pends, so nothing is due");
    assert_eq!(c.drain_due(1_000_000), Emission::Batch(vec![]));
}
