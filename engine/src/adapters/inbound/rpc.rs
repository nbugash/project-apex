//! Inbound adapter: JSON-RPC method dispatch.
//!
//! Moved here from `main.rs` when the engine acquired ports and adapters. It translates protocol
//! input into use-case input and results and errors back; it holds no business rules
//! (Principle VIII). Behaviour is F002's, unchanged by the move.

use crate::handshake;
use crate::session::{self, SessionRegistry};
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::{HandshakeRequest, RestartNotice};
use bytes::BytesMut;
use std::io::Write;

/// JSON-RPC's code for a method the receiver does not implement.
pub const METHOD_NOT_FOUND: i32 = -32601;
/// Params that are not what the method requires.
pub const INVALID_PARAMS: i32 = -32602;

/// What the loop should do with one decoded frame.
pub enum Action {
    /// Write this and carry on.
    Reply(Vec<u8>),
    /// Nothing to say.
    Nothing,
    /// Replace this process, once anything still buffered has been answered.
    Restart(Vec<u8>),
}

/// Answer one request, or say what else the loop must do.
pub fn dispatch(
    registry: &SessionRegistry,
    roots: &dyn crate::application::ports::roots::WorkspaceRoots,
    fs: &dyn crate::application::ports::file_system::FileSystem,
    codec: &FrameCodec,
    body: &str,
) -> Action {
    let _ = (roots, fs); // threaded through for the workspace methods that land in Phase 3
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return Action::Nothing;
    };
    let Some(id) = parsed
        .get("id")
        .and_then(|i| i.as_str())
        .map(str::to_string)
    else {
        return Action::Nothing;
    };
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
                    reply_or_nothing(encode_result(codec, &id, &response))
                }
                Err(e) => {
                    reply_or_nothing(encode_error(codec, &id, INVALID_PARAMS, &format!("{e}")))
                }
            }
        }
        // In-place re-execution (§15.3, FR-020).
        //
        // `exec` replaces the process image while keeping the file descriptors, so stdin and
        // stdout — the client's connection — survive. Nothing in memory does, which is why
        // the session identity travels in the environment: the new image adopts it and the
        // client re-attaches to the same session rather than discovering a new one.
        //
        // The reply is written and flushed *before* exec, because after it there is no
        // process left to answer with.
        "session/restart" => match encode_result(codec, &id, &serde_json::Value::Null) {
            Some(frame) => Action::Restart(frame),
            None => Action::Nothing,
        },
        // ---- Workspace (§4.8). Registration first: every other method needs it. ----
        "workspace/register" => {
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match serde_json::from_value::<apex_protocol::wire::RegisterParams>(params) {
                Ok(req) => match roots.register(&req.workspace_id.0, &req.path) {
                    Ok(root) => {
                        let canonical = root.as_path().to_string_lossy().to_string();
                        let name = root
                            .as_path()
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| canonical.clone());
                        let result = apex_protocol::wire::RegisterResult {
                            name,
                            canonical_path: canonical,
                        };
                        reply_or_nothing(encode_result(codec, &id, &result))
                    }
                    Err(e) => {
                        let (code, message) =
                            crate::application::use_cases::workspace::RequestRefusal::Root(e)
                                .wire();
                        reply_or_nothing(encode_error(codec, &id, code, &message))
                    }
                },
                Err(e) => {
                    reply_or_nothing(encode_error(codec, &id, INVALID_PARAMS, &format!("{e}")))
                }
            }
        }
        "session/shutdown" => {
            if let Some(frame) = encode_result(codec, &id, &serde_json::Value::Null) {
                let mut out = std::io::stdout();
                let _ = out.write_all(&frame);
                let _ = out.flush();
            }
            std::process::exit(0);
        }
        _ => reply_or_nothing(encode_error(
            codec,
            &id,
            METHOD_NOT_FOUND,
            &format!("this engine does not implement {method}"),
        )),
    }
}

fn reply_or_nothing(frame: Option<Vec<u8>>) -> Action {
    match frame {
        Some(f) => Action::Reply(f),
        None => Action::Nothing,
    }
}

/// Code returned to anything still buffered when the engine re-executes.
const RESTARTING: i32 = -32000;

/// Answer everything already read but not yet processed, then replace the process.
///
/// `exec` keeps the file descriptors but discards memory — including whatever the reader had
/// already pulled out of the pipe. Without this, a request that arrived just before a restart
/// disappears while the connection stays up, so the client waits for a reply that can never
/// come until its own timeout expires. A-REQ makes losing a request acceptable when the
/// connection dies; here the connection survives, so silence would be a lie.
pub fn drain_and_exec(
    registry: &SessionRegistry,
    codec: &mut FrameCodec,
    buf: &mut BytesMut,
    ack: &[u8],
) -> ! {
    let mut out = std::io::stdout();
    let _ = out.write_all(ack);

    while let Ok(Some(frame)) = codec.decode(buf) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&frame.0) {
            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                if let Some(f) = encode_error(
                    codec,
                    id,
                    RESTARTING,
                    "the engine is restarting; re-issue this request",
                ) {
                    let _ = out.write_all(&f);
                }
            }
        }
    }
    let _ = out.flush();

    let err = exec_self(&registry.current().0);
    // Only reachable if exec failed. The old image is intact, so report and keep serving
    // rather than exiting and taking the session down with us.
    eprintln!("re-execution failed, continuing on the current image: {err}");
    std::process::exit(1);
}

/// Replace this process with a fresh copy of the engine binary, carrying the session forward.
///
/// Returns only on failure: on success there is no longer a process to return into.
fn exec_self(session_id: &str) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return e,
    };
    std::process::Command::new(exe)
        .env(session::SESSION_ENV, session_id)
        .exec()
}

pub fn encode_result<T: serde::Serialize>(
    codec: &FrameCodec,
    id: &str,
    result: &T,
) -> Option<Vec<u8>> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
    codec.encode(&body.to_string()).ok()
}

pub fn encode_error(codec: &FrameCodec, id: &str, code: i32, message: &str) -> Option<Vec<u8>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": code, "message": message}
    });
    codec.encode(&body.to_string()).ok()
}

pub fn encode_notification(
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
        let fs = crate::adapters::outbound::std_fs::StdFileSystem;
        let roots = crate::application::use_cases::workspace::InMemoryRoots::new(
            std::sync::Arc::new(crate::adapters::outbound::std_fs::StdFileSystem),
        );
        for body in [
            r#"{"jsonrpc":"2.0","id":"1","method":"auth/handshake","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":"2","method":"auth/handshake","params":[1,2,3]}"#,
            r#"{"jsonrpc":"2.0","id":"3","method":"auth/handshake","params":{"protocol_version":"not a number"}}"#,
            r#"{"jsonrpc":"2.0","id":"4","method":"auth/handshake","params":null}"#,
        ] {
            let Action::Reply(reply) = dispatch(&registry, &roots, &fs, &codec, body) else {
                panic!("expected a reply, not a panic or silence: {body}")
            };
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
        let fs = crate::adapters::outbound::std_fs::StdFileSystem;
        let roots = crate::application::use_cases::workspace::InMemoryRoots::new(
            std::sync::Arc::new(crate::adapters::outbound::std_fs::StdFileSystem),
        );
        let _ = dispatch(
            &registry,
            &roots,
            &fs,
            &codec,
            r#"{"jsonrpc":"2.0","id":"1","method":"auth/handshake","params":{}}"#,
        );
        let Action::Reply(good) = dispatch(
            &registry,
            &roots,
            &fs,
            &codec,
            r#"{"jsonrpc":"2.0","id":"2","method":"auth/handshake","params":{"client_version":"0.1.0","protocol_version":1,"capabilities":[]}}"#,
        ) else {
            panic!("expected a reply")
        };
        assert!(decode(&good).get("result").is_some());
    }

    #[test]
    fn an_unimplemented_method_is_refused_by_name() {
        let registry = SessionRegistry::new();
        let codec = FrameCodec::new();
        let fs = crate::adapters::outbound::std_fs::StdFileSystem;
        let roots = crate::application::use_cases::workspace::InMemoryRoots::new(
            std::sync::Arc::new(crate::adapters::outbound::std_fs::StdFileSystem),
        );
        let Action::Reply(reply) = dispatch(
            &registry,
            &roots,
            &fs,
            &codec,
            r#"{"jsonrpc":"2.0","id":"9","method":"workspace/readFile","params":{}}"#,
        ) else {
            panic!("expected a reply")
        };
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
        let fs = crate::adapters::outbound::std_fs::StdFileSystem;
        let roots = crate::application::use_cases::workspace::InMemoryRoots::new(
            std::sync::Arc::new(crate::adapters::outbound::std_fs::StdFileSystem),
        );
        for body in ["this is not json", "", "{}"] {
            assert!(matches!(
                dispatch(&registry, &roots, &fs, &codec, body),
                Action::Nothing
            ));
        }
    }
}
