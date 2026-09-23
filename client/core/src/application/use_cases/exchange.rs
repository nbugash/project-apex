//! The rules of a request exchange, apart from the pipe that carries it (User Story 3).
//!
//! What lives here is protocol and policy: the shape of a request on the wire, the shape of
//! a cancellation, and how long a request may wait. What stays in the adapter is what
//! genuinely needs a child process — framing bytes, the send queue, the reader thread.
//!
//! The split matters because these three are facts the *next* feature has to match. Left
//! inline in the transport they would be copied by whoever needs them next, and a copied
//! protocol detail is one that drifts silently: two envelopes that differ by a field name
//! fail at the far end, not here.

use crate::application::ports::transport::Request;
use crate::domain::request::{RequestId, DEFAULT_TIMEOUT_SECS};
use std::time::Duration;

/// The JSON-RPC 2.0 method that withdraws a request in flight (§4.5).
///
/// A notification, not a request: it carries no id of its own and expects no reply. Waiting
/// for an acknowledgement would mean a withdrawal could itself time out, and the caller has
/// already stopped caring about the work — that is what withdrawing means.
pub const CANCEL_METHOD: &str = "$/cancelRequest";

/// A zero default would expire every request before it was written. Checked at compile
/// time rather than in a test, because a test can only fail after the build succeeded.
const _: () = assert!(DEFAULT_TIMEOUT_SECS > 0);

/// How long this request may wait before it resolves as `TimedOut` (FR-012).
///
/// Every request has a limit. One without would wait forever on a link that is up but
/// unresponsive, which is indistinguishable from a hang to the user and invisible to the
/// supervisor — the request is not lost, so nothing reports it.
pub fn deadline_for(request: &Request) -> Duration {
    request
        .timeout
        .unwrap_or(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
}

/// The request as it goes on the wire.
///
/// `params` is passed through uninterpreted. What is inside a frame is the business of the
/// feature that sent it; a transport that parsed payloads would need updating for every
/// method any future feature adds.
pub fn request_body(id: &RequestId, request: &Request) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":"{id}","method":"{}","params":{}}}"#,
        request.method, request.params
    )
}

/// The cancellation notification for a request being withdrawn (§4.5).
pub fn cancellation_body(id: &RequestId) -> String {
    format!(r#"{{"jsonrpc":"2.0","method":"{CANCEL_METHOD}","params":{{"id":"{id}"}}}}"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::request::Priority;

    fn req() -> Request {
        Request::interactive("textDocument/completion", r#"{"line":4}"#)
    }

    #[test]
    fn a_request_without_a_stated_limit_takes_the_default() {
        assert_eq!(
            deadline_for(&req()),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS)
        );
    }

    /// A caller that knows its own budget states it. A completion popup that is useless
    /// after 300 ms should not hold a registry entry for thirty seconds.
    #[test]
    fn a_caller_may_state_a_shorter_limit() {
        let mut r = req();
        r.timeout = Some(Duration::from_millis(300));
        assert_eq!(deadline_for(&r), Duration::from_millis(300));
    }

    #[test]
    fn the_request_body_carries_the_id_the_reply_will_be_matched_by() {
        let id = RequestId::new(7);
        let body = request_body(&id, &req());
        assert!(body.contains(&format!(r#""id":"{id}""#)), "{body}");
        assert!(body.contains(r#""jsonrpc":"2.0""#), "{body}");
        assert!(
            body.contains(r#""method":"textDocument/completion""#),
            "{body}"
        );
    }

    /// Params are not re-encoded. A transport that parsed and re-serialised them would need
    /// a schema for every method any future feature invents.
    #[test]
    fn params_are_passed_through_untouched() {
        let r = Request::background("engine/index", r#"{"paths":["a","b"],"deep":{"n":1}}"#);
        assert!(
            request_body(&RequestId::new(1), &r)
                .contains(r#""params":{"paths":["a","b"],"deep":{"n":1}}"#),
            "params must reach the engine exactly as the caller wrote them"
        );
        assert_eq!(r.priority, Priority::Background);
    }

    /// A notification: no id of its own. With one, the withdrawal would itself be a request
    /// awaiting a reply, and could time out — while the caller has by definition stopped
    /// waiting for anything.
    #[test]
    fn a_cancellation_is_a_notification_naming_the_request_it_withdraws() {
        let id = RequestId::new(42);
        let body = cancellation_body(&id);
        assert!(
            body.contains(&format!(r#""method":"{CANCEL_METHOD}""#)),
            "{body}"
        );
        assert!(body.contains(&format!(r#""id":"{id}""#)), "{body}");
        assert!(
            !body.contains(r#""id":"$/"#),
            "the cancellation must not carry an id of its own: {body}"
        );
        // The id inside params is the withdrawn request's; the notification itself has none
        // at the top level.
        let top_level_id = body.split(r#""params""#).next().unwrap_or_default();
        assert!(
            !top_level_id.contains(r#""id""#),
            "a notification with a top-level id is a request: {body}"
        );
    }
}
