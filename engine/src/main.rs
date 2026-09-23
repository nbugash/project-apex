//! `ide-engine` — the remote half of the product.
//!
//! F002 builds the first real one. It answers the handshake, owns session identity and reports
//! its own restarts, and implements no workspace method: F003 adds those. Until this binary
//! existed the far end of every connection was a mock forbidden to implement any §4.8 method,
//! which is why `auth/handshake` had no possible responder.
//!
//! **Everything arriving on stdin is untrusted** (Constitution Principle VI). The engine runs
//! with the developer's full rights on their own machine, so a malformed frame must produce a
//! refusal rather than a panic, and must never leave the reader misaligned — a desynchronised
//! stream turns one bad message into every later message being garbage, which presents as the
//! engine having gone insane.

mod handshake;
mod session;

use apex_protocol::framing::{FrameCodec, FrameError};
use apex_protocol::wire::{HandshakeRequest, RestartNotice};
use bytes::BytesMut;
use session::SessionRegistry;
use std::io::{Read, Write};

/// JSON-RPC's code for a method the receiver does not implement.
const METHOD_NOT_FOUND: i32 = -32601;
/// Params that are not what the method requires.
const INVALID_PARAMS: i32 = -32602;

fn main() {
    let registry = SessionRegistry::new();
    let mut codec = FrameCodec::new();
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    // A restart is announced, never inferred. The client learns about it because it was told,
    // and the identity it carries is what distinguishes a restart from a new session.
    if registry.restarted() {
        let notice = registry.restart_notice();
        if let Some(frame) = encode_notification(&codec, "session/onRestart", &notice) {
            let _ = stdout.write_all(&frame);
            let _ = stdout.flush();
        }
    }

    let mut buf = BytesMut::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stdin.read(&mut chunk) {
            Ok(0) | Err(_) => return, // the client went away
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(frame)) => {
                    if let Some(reply) = dispatch(&registry, &codec, &frame.0) {
                        let _ = stdout.write_all(&reply);
                        let _ = stdout.flush();
                    }
                }
                Ok(None) => break,
                // A refused frame is refused alone. The codec has already left the buffer at a
                // boundary, so the next frame still reads.
                Err(FrameError::TooLarge(n)) => {
                    eprintln!("refused a frame declaring {n} bytes");
                }
                Err(FrameError::Malformed(why)) => {
                    eprintln!("discarded a frame: {why}");
                }
            }
        }
    }
}

/// Answer one request, or `None` for a notification that needs no reply.
fn dispatch(registry: &SessionRegistry, codec: &FrameCodec, body: &str) -> Option<Vec<u8>> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let id = parsed.get("id")?.as_str()?.to_string();
    let method = parsed.get("method").and_then(|m| m.as_str()).unwrap_or("");

    match method {
        "auth/handshake" => {
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // A well-framed but malformed payload is refused, not fatal. The engine keeps
            // serving: one bad request must not end a session.
            match serde_json::from_value::<HandshakeRequest>(params) {
                Ok(request) => {
                    let response = handshake::respond(registry, &request);
                    encode_result(codec, &id, &response)
                }
                Err(e) => encode_error(codec, &id, INVALID_PARAMS, &format!("{e}")),
            }
        }
        "session/shutdown" => {
            let reply = encode_result(codec, &id, &serde_json::Value::Null);
            if let Some(frame) = &reply {
                let mut out = std::io::stdout();
                let _ = out.write_all(frame);
                let _ = out.flush();
            }
            std::process::exit(0);
        }
        _ => encode_error(
            codec,
            &id,
            METHOD_NOT_FOUND,
            &format!("this engine does not implement {method}"),
        ),
    }
}

fn encode_result<T: serde::Serialize>(codec: &FrameCodec, id: &str, result: &T) -> Option<Vec<u8>> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
    codec.encode(&body.to_string()).ok()
}

fn encode_error(codec: &FrameCodec, id: &str, code: i32, message: &str) -> Option<Vec<u8>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": code, "message": message}
    });
    codec.encode(&body.to_string()).ok()
}

fn encode_notification(
    codec: &FrameCodec,
    method: &str,
    params: &RestartNotice,
) -> Option<Vec<u8>> {
    let body = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
    codec.encode(&body.to_string()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(frame: &[u8]) -> serde_json::Value {
        let text = String::from_utf8_lossy(frame);
        let body = text.split_once("\r\n\r\n").expect("a framed reply").1;
        serde_json::from_str(body).expect("json")
    }

    /// T080. Principle VI: a well-framed but malformed payload is refused without panicking and
    /// without the engine acting on any part of it. The codec tests cover framing; nothing
    /// before this covered payloads.
    #[test]
    fn a_malformed_handshake_payload_is_refused_rather_than_fatal() {
        let registry = SessionRegistry::new();
        let codec = FrameCodec::new();
        for body in [
            r#"{"jsonrpc":"2.0","id":"1","method":"auth/handshake","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":"2","method":"auth/handshake","params":[1,2,3]}"#,
            r#"{"jsonrpc":"2.0","id":"3","method":"auth/handshake","params":{"protocol_version":"not a number"}}"#,
            r#"{"jsonrpc":"2.0","id":"4","method":"auth/handshake","params":null}"#,
        ] {
            let reply = dispatch(&registry, &codec, body).expect("a reply, not a panic");
            let v = decode(&reply);
            assert_eq!(
                v["error"]["code"], INVALID_PARAMS,
                "malformed params must be refused: {body}"
            );
            assert!(v.get("result").is_none(), "nothing may be acted on: {body}");
        }
    }

    /// The engine keeps serving after refusing one bad request. A session ending because of a
    /// single malformed message would turn a client bug into an outage.
    #[test]
    fn a_refused_request_does_not_end_the_session() {
        let registry = SessionRegistry::new();
        let codec = FrameCodec::new();
        let _ = dispatch(
            &registry,
            &codec,
            r#"{"jsonrpc":"2.0","id":"1","method":"auth/handshake","params":{}}"#,
        );
        let good = dispatch(
            &registry,
            &codec,
            r#"{"jsonrpc":"2.0","id":"2","method":"auth/handshake","params":{"client_version":"0.1.0","protocol_version":1,"capabilities":[]}}"#,
        )
        .expect("a reply");
        assert!(decode(&good).get("result").is_some());
    }

    #[test]
    fn an_unimplemented_method_is_refused_by_name() {
        let registry = SessionRegistry::new();
        let codec = FrameCodec::new();
        let reply = dispatch(
            &registry,
            &codec,
            r#"{"jsonrpc":"2.0","id":"9","method":"workspace/readFile","params":{}}"#,
        )
        .expect("a reply");
        let v = decode(&reply);
        assert_eq!(v["error"]["code"], METHOD_NOT_FOUND);
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("workspace/readFile"),
            "the refusal must name what was asked for"
        );
    }

    /// Garbage that is not JSON at all yields no reply and no panic — there is no id to answer.
    #[test]
    fn a_body_that_is_not_json_is_dropped_without_panicking() {
        let registry = SessionRegistry::new();
        let codec = FrameCodec::new();
        assert!(dispatch(&registry, &codec, "this is not json").is_none());
        assert!(dispatch(&registry, &codec, "").is_none());
        assert!(dispatch(&registry, &codec, "{}").is_none());
    }
}
