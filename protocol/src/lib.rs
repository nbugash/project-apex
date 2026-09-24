//! The wire format shared by the client and the engine.
//!
//! §4.1 defines `Content-Length` framing normatively and exactly. This crate exists so there is
//! one implementation of it rather than two: a framing disagreement between the two ends
//! corrupts every message rather than failing one, and the mock daemon is deliberately
//! restricted to framing for the same reason — to bound how far a double can drift from the
//! real thing.
//!
//! This does not weaken F001's rule that nothing outside the transport adapter may depend on
//! the codec. That rule keeps the application layer from reaching around its port, and it still
//! holds. Sharing a format with the process at the other end of the wire is what a protocol is.

pub mod base64;
pub mod framing;
pub mod wire;

pub use framing::{FrameCodec, FrameError};
