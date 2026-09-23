//! `ide-engine` as a library, so its behaviour is testable without driving a process.
//!
//! Organised as ports and adapters (Principle VIII). F002 built this binary as a stdio loop with
//! a `match` on the method name — correct while there were three session methods with no logic
//! behind them. F003 gives the engine its first real behaviour: path canonicalisation, directory
//! paging, ranged reads. The moment there is logic to keep out of the adapter is the moment the
//! principle starts to cost something, and deferring it would mean F004's watcher and F013's
//! search extend a shape the constitution already rejects.
//!
//! **Everything arriving on stdin is untrusted** (Principle VI). The engine runs with the
//! developer's full filesystem rights, so a malformed frame must produce a refusal rather than a
//! panic, and must never leave the reader misaligned.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod handshake;
pub mod session;
