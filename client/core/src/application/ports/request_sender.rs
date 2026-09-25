//! Outbound port: send one request, get one outcome — object-safe.
//!
//! F001's `RequestTransport` uses native `async fn` in traits, which is right there: it is
//! selected once at composition by a generic parameter and never varies at a call site. But a
//! generic `T: RequestTransport` gives the compiler no way to prove the returned future is
//! `Send`, and `WorkspaceProvider` must be `dyn` **and** `Send` because FR-001 requires consumers
//! not to know which implementation they hold.
//!
//! This shim is the seam between those two facts. A concrete adapter implements it, so the
//! compiler checks `Send` against a future it can actually see, and the workspace layer depends
//! on an object-safe capability rather than on a generic parameter it would have to spread
//! through every holder.

use crate::application::ports::transport::Request;
use crate::domain::request::RequestOutcome;
use async_trait::async_trait;

#[async_trait]
pub trait RequestSender: Send + Sync {
    /// Exactly one outcome, exactly once. Never panics.
    async fn send(&self, request: Request) -> RequestOutcome;

    /// Send a JSON-RPC **notification**: no id, no reply, no outcome.
    ///
    /// A separate method rather than a flag on `Request`, because the difference is not a
    /// property of the message but of what the caller may do next. §4.2 gives a notification no
    /// response at all, so putting one through `send` would wait for a reply that is never
    /// coming -- and the caller would have no way to tell that from a slow engine.
    ///
    /// It follows that an unknown task, an exited task and a full buffer are all indistinguishable
    /// to the sender. That is the contract, not a shortcoming of this signature: there is no
    /// response to carry a refusal in.
    async fn notify(&self, request: Request);
}
