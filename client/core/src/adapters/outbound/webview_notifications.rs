//! Carry engine-initiated frames into the webview.
//!
//! One Tauri event, `apex:notification`, carrying the method and the frame untouched. The
//! webview routes on the method.
//!
//! # Why one event and not one per method
//!
//! An event per method reads better until something has to add a method, at which point the
//! addition is in three places -- the engine, this file, and the listener -- and forgetting the
//! middle one produces silence rather than an error. With one event a new method reaches the
//! webview whether or not anybody remembered this file, and an unrouted method is visible there
//! as an unhandled case rather than invisible here as an absent branch.
//!
//! # Why the frame goes across uninterpreted
//!
//! Parsing here would make this a second place that has to agree with the wire format, and the
//! webview already agrees with it -- `applyChunk` reads `data` itself. Two parsers that must
//! stay in step is how a client and an engine come to disagree about a field name while each
//! looks correct alone.
//!
//! # What this must not do
//!
//! It runs on the transport's reader thread (see `NotificationSink`), so blocking here stops
//! the engine's output being drained, which is the backpressure the whole path depends on.
//! `emit` serialises and hands off; it does not wait for the webview to render.

use tauri::{AppHandle, Emitter};

use crate::application::ports::notification_sink::NotificationSink;

/// The single event every engine-initiated frame arrives on.
pub const NOTIFICATION_EVENT: &str = "apex:notification";

#[derive(Clone, serde::Serialize)]
struct Payload<'a> {
    method: &'a str,
    /// The whole JSON-RPC frame, as it came off the wire.
    body: &'a str,
}

pub struct WebviewNotifications {
    app: AppHandle,
}

impl WebviewNotifications {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl NotificationSink for WebviewNotifications {
    fn deliver(&self, method: &str, body: &str) {
        if let Err(e) = self.app.emit(NOTIFICATION_EVENT, Payload { method, body }) {
            // Logged once per failure rather than swallowed: a webview that has gone away is
            // normal during shutdown, and a task whose output stops arriving for any other
            // reason is precisely the failure this whole path exists to prevent. Silence here
            // would make the two indistinguishable, which is the state this feature started in.
            crate::logging::warn(&format!("{method} did not reach the webview: {e}"));
        }
    }
}
