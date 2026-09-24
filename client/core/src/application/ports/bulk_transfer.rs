//! Outbound port: move bytes beside the protocol channel (A-BULK, §3.6).
//!
//! Anything above A-BULKSIZE's threshold takes its own `ssh` invocation on the existing control
//! master rather than being chunked through the control pipe, which §4.6 forbids: one pipe is one
//! queue, and a large response serialises ahead of every interactive request behind it.

use crate::domain::workspace::ByteRange;
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkError {
    NotFound,
    /// The invocation failed, or produced fewer bytes than the range asked for.
    Transfer(String),
}

#[async_trait]
pub trait BulkTransfer: Send + Sync {
    /// Fetch an absolute path on the engine's host.
    ///
    /// Returns bytes and **no integrity claim**: nothing about the transfer authenticates the
    /// content, so the caller compares against the digest from `stat` and discards a mismatch.
    async fn fetch(&self, remote: &str, range: Option<ByteRange>) -> Result<Vec<u8>, BulkError>;
}
