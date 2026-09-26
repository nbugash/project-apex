//! Where an engine-initiated frame goes.
//!
//! JSON-RPC is bidirectional and the engine uses that: `execution/onStdout`, `execution/onExit`
//! and `workspace/onFileEvent` all arrive unsolicited, carrying no id to correlate. Until this
//! port existed the transport dropped them, and both halves of that were true at once -- the
//! reply-matching function has nothing to do with a frame that is not a reply, and nothing else
//! was looking, so a task's entire output went nowhere.
//!
//! # Called on the transport's reader thread, deliberately
//!
//! A sink that blocks stops the reader, which stops draining the engine's stdout, which is
//! exactly the backpressure FR-013 describes: the engine's frame writer blocks on its pipe, its
//! reader thread stops reading the pseudo-terminal, the terminal's buffer fills, and the task
//! blocks in `write`. Handing the frame to an unbounded queue here would buffer between the
//! producer and the consumer and break that chain -- the one property the whole output path is
//! built around, and the reason the engine has no queue of its own.
//!
//! So an implementation does the least it can and returns. Forwarding to the webview is a
//! bounded write; parsing, rendering or persisting here would not be.

/// One engine-initiated frame.
pub trait NotificationSink: Send + Sync {
    /// `method` is the JSON-RPC method name; `body` is the whole frame, uninterpreted.
    ///
    /// The body rather than the extracted params, because a sink's job is to forward rather
    /// than to understand. Every consumer so far re-serialises what it is handed, and a sink
    /// that parsed would become a second place that has to agree with the wire -- which is how
    /// a client and an engine come to disagree about a field name without either being wrong
    /// on its own terms.
    fn deliver(&self, method: &str, body: &str);
}

/// Drops everything, which is what the transport did before this port existed.
///
/// A named type rather than an `Option` threaded through the transport: a transport with no
/// interest in notifications is an ordinary configuration -- F001's own suite has none -- and
/// making every construction site say `None` buys nothing over saying so once here.
pub struct DiscardNotifications;

impl NotificationSink for DiscardNotifications {
    fn deliver(&self, _method: &str, _body: &str) {}
}
