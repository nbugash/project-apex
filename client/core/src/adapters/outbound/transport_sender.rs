//! The adapter that was missing: a `RequestSender` backed by a real transport.
//!
//! `RemoteWorkspaceProvider` and `RemoteTasks` were both written, both tested against doubles,
//! and neither could ever be constructed, because nothing in this crate implemented the port
//! they depend on. The engine-backed half of the application was complete and unreachable.
//!
//! # Why the port exists at all, when `RequestTransport` is right there
//!
//! `request_sender.rs` has the reasoning and it is worth not re-deriving: `RequestTransport`
//! uses native `async fn` in traits, which is correct for something selected once at
//! composition by a generic parameter, but gives the compiler no way to prove the returned
//! future is `Send`. A provider must be `dyn` **and** `Send`. This type is the seam between
//! those two facts -- a concrete adapter, so `Send` is checked against a future the compiler
//! can see.

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::outbound::openssh::SshTransport;
use crate::application::ports::request_sender::RequestSender;
use crate::application::ports::transport::{Request, RequestTransport};
use crate::domain::request::RequestOutcome;

/// Concrete in `SshTransport`, and that is the point rather than a missed generalisation.
///
/// A `TransportSender<T: RequestTransport>` is the obvious shape and does not compile: the
/// port's `send` is a native `async fn` in a trait, so for an unknown `T` the compiler cannot
/// prove the returned future is `Send`, and `RequestSender` must be `dyn` **and** `Send`. Naming
/// the one implementation is what lets it see the future it is checking. A second transport
/// would add a second small adapter here, which is cheaper than the alternative -- making every
/// holder of a provider generic over the transport underneath it.
pub struct TransportSender {
    transport: Arc<SshTransport>,
}

impl TransportSender {
    pub fn new(transport: Arc<SshTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl RequestSender for TransportSender {
    async fn send(&self, request: Request) -> RequestOutcome {
        // `begin` rather than `send`, because `begin` hands back a `Pending`, which is declared
        // `+ Send`. The port's `send` is a native async fn whose future carries no such
        // guarantee, and awaiting it here is what the compiler refuses.
        let (_id, pending) = self.transport.begin(request);
        pending.await
    }

    /// Forwarded, not simulated.
    ///
    /// The obvious shortcut is to send a notification through `send` and discard the outcome,
    /// which compiles and is wrong twice over: it puts an id on a frame §4.2 says has none, so
    /// a conforming engine would reply to something the client has stopped listening for; and
    /// it holds a registry slot for the full timeout on every keystroke.
    async fn notify(&self, request: Request) {
        self.transport.notify(request);
    }
}
