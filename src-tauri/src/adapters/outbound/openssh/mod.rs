//! The OpenSSH-backed transport.
//!
//! Its internals are private modules: nothing outside this directory may depend on the
//! codec, the registry or the send queue. The application layer sees `RequestTransport` and
//! nothing else, which is what makes the mock a drop-in rather than a parallel
//! implementation.

mod classify;
mod framing;
mod registry;
mod sendq;
mod spawner;
