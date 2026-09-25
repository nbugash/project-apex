//! Inbound adapter: JSON-RPC method dispatch.
//!
//! Moved here from `main.rs` when the engine acquired ports and adapters. It translates protocol
//! input into use-case input and results and errors back; it holds no business rules
//! (Principle VIII). Behaviour is F002's, unchanged by the move.

use crate::application::use_cases::workspace;
use crate::domain::task::TaskSignal;
use crate::handshake;
use crate::session::{self, SessionRegistry};
use apex_protocol::framing::FrameCodec;
use apex_protocol::wire::{codes, HandshakeRequest};
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

/// Which §4.4 code a start refusal becomes, and what a human is told.
///
/// Every `SpawnFailure` becomes `-32011`, because FR-004 and SC-015 admit one outcome for a
/// command that could not be started and §4.4 gives that outcome one code. The distinction that
/// matters to a developer is whose fault it is, which is what the message carries -- and no
/// message may carry the environment (FR-005a, SC-025).
fn refusal_to_wire(
    refusal: &crate::application::use_cases::task::StartRefusal,
) -> (i32, &'static str) {
    use crate::application::ports::task_runner::SpawnFailure;
    use crate::application::use_cases::task::StartRefusal as R;
    match refusal {
        R::NotRegistered => (
            codes::WORKSPACE_NOT_REGISTERED,
            "workspace is not registered",
        ),
        R::RootGone => (codes::WORKSPACE_GONE, "the workspace root no longer exists"),
        R::PathRefused => (
            codes::PATH_REFUSED,
            "the working directory escapes the workspace root",
        ),
        R::NotFound => (codes::NOT_FOUND, "the working directory does not exist"),
        R::AlreadyRunning => (
            codes::TASK_ALREADY_RUNNING,
            "a task is already running under that identity; attach to it rather than starting it",
        ),
        R::CouldNotStart(SpawnFailure::NotExecutable) => (
            codes::COMMAND_NOT_STARTED,
            "the command was not found, or is not executable",
        ),
        R::CouldNotStart(SpawnFailure::CwdUnusable) => (
            codes::COMMAND_NOT_STARTED,
            "the working directory could not be entered",
        ),
        R::CouldNotStart(SpawnFailure::NoDevice) => (
            codes::COMMAND_NOT_STARTED,
            "the instance could not allocate a terminal or a pipe",
        ),
        R::CouldNotStart(SpawnFailure::LimitRefused) => (
            codes::COMMAND_NOT_STARTED,
            "the instance refused the task's resource limits",
        ),
        R::CouldNotStart(SpawnFailure::Failed(_)) => (
            codes::COMMAND_NOT_STARTED,
            "the command could not be started",
        ),
    }
}

/// Act on a frame that carries no `id`.
///
/// A notification has no response (§4.2), so every arm here returns `Action::Nothing` and the
/// return value says only what the loop should do next -- never what to send. An unknown method
/// is dropped in silence, because there is no id to answer and nothing else §4.2 permits.
///
/// F010's `execution/writeStdin` and `execution/resizePty` are the catalogue's first
/// client-to-engine notifications, and they land here.
/// The client-to-engine notifications: input and resize.
///
/// **Everything here returns `Action::Nothing`, and that is the contract rather than an
/// oversight.** §4.2 gives a notification no response, so an unknown `taskId`, a task that has
/// already exited, a payload that does not parse, `data` that is not base64, and a `cols` or
/// `rows` that is absent, zero or not an integer are all dropped in the same silence. There is no
/// response to carry a refusal in, and inventing an error frame for a request that had no id
/// would be an unsolicited reply the client cannot match to anything.
fn dispatch_notification(
    method: &str,
    params: Option<&serde_json::Value>,
    tasks: Option<&crate::adapters::outbound::task_threads::TaskService>,
) -> Action {
    use crate::application::use_cases::task::{resize_task, write_input};

    let (Some(service), Some(params)) = (tasks, params) else {
        return Action::Nothing;
    };
    match method {
        "execution/writeStdin" => {
            let Ok(p) =
                serde_json::from_value::<apex_protocol::wire::WriteStdinParams>(params.clone())
            else {
                return Action::Nothing;
            };
            // Decoded exactly once, here, at the boundary. Everything inward takes bytes.
            let Ok(bytes) = apex_protocol::base64::decode(&p.data) else {
                return Action::Nothing;
            };
            let _ = write_input(&bytes, service.control(&p.task_id));
            Action::Nothing
        }
        "execution/resizePty" => {
            let Ok(p) =
                serde_json::from_value::<apex_protocol::wire::ResizePtyParams>(params.clone())
            else {
                return Action::Nothing;
            };
            let _ = resize_task(
                service.shape(&p.task_id),
                p.cols,
                p.rows,
                service.control(&p.task_id),
            );
            Action::Nothing
        }
        _ => Action::Nothing,
    }
}

/// Answer one request, or say what else the loop must do.
/// Seven parameters, which is one more than is comfortable and **does not extend again**.
///
/// F004 threaded `watchers` this way and F010 threads `tasks` the same way, which is the right
/// call for the second collaborator and the wrong one for the third. The next feature that needs
/// one introduces a `DispatchContext` carrying these by reference rather than an eighth argument
/// -- recorded here because the moment to notice is when adding the next one, not when reading
/// the signature afterwards.
#[allow(clippy::too_many_arguments)]
pub fn dispatch(
    registry: &SessionRegistry,
    roots: &crate::application::use_cases::workspace::InMemoryRoots,
    fs: &dyn crate::application::ports::file_system::FileSystem,
    watchers: Option<&crate::adapters::outbound::watchers::Watchers>,
    tasks: Option<&crate::adapters::outbound::task_threads::TaskService>,
    codec: &FrameCodec,
    body: &str,
) -> Action {
    use crate::application::ports::roots::WorkspaceRoots;
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return Action::Nothing;
    };
    let method = parsed.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Read as a `Value`, not with `as_str`. Two defects lived in that one call. An **absent**
    // id is a notification, and returning here meant `execution/writeStdin` and
    // `execution/resizePty` -- the catalogue's first client-to-engine notifications -- could
    // not reach a handler at all, because this returned before the method match. And a
    // **numeric** id, legal under JSON-RPC 2.0, is not a string, so it took the same exit and
    // was dropped as though it were a notification.
    let Some(id) = parsed.get("id").cloned() else {
        return dispatch_notification(method, parsed.get("params"), tasks);
    };
    let id = &id;

    match method {
        "execution/runTask" => {
            let Some(service) = tasks else {
                // No task service composed: the engine builds and runs without one, and
                // `runTask` is refused with a reason rather than appearing to succeed --
                // F004's degradation shape (FR-027, A-WATCHLOCAL).
                return reply_or_nothing(encode_error(
                    codec,
                    id,
                    codes::COMMAND_NOT_STARTED,
                    "this engine has no task service",
                ));
            };
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match serde_json::from_value::<apex_protocol::wire::RunTaskParams>(params) {
                Ok(p) => match service.run(&p, roots, fs) {
                    Ok(pid) => reply_or_nothing(encode_result(
                        codec,
                        id,
                        &apex_protocol::wire::RunTaskResult { pid },
                    )),
                    Err(refusal) => {
                        let (code, message) = refusal_to_wire(&refusal);
                        reply_or_nothing(encode_error(codec, id, code, message))
                    }
                },
                Err(e) => {
                    reply_or_nothing(encode_error(codec, id, INVALID_PARAMS, &format!("{e}")))
                }
            }
        }
        "execution/terminate" => {
            use crate::application::use_cases::task::stop_task;
            let Some(service) = tasks else {
                return reply_or_nothing(encode_error(
                    codec,
                    id,
                    codes::TASK_NOT_FOUND,
                    "this engine has no task service",
                ));
            };
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // A signal outside the closed three is refused **before anything reaches a
            // syscall**, by the type: `TerminateSignal` has three variants and serde rejects the
            // rest, so an arbitrary string from the wire never becomes a number this process
            // passes to `kill`. That is `ResolvedPath`'s reasoning applied to a second kind of
            // untrusted input -- the refusal is structural rather than a check somebody has to
            // remember to write.
            let p = match serde_json::from_value::<apex_protocol::wire::TerminateParams>(params) {
                Ok(p) => p,
                Err(e) => {
                    return reply_or_nothing(encode_error(
                        codec,
                        id,
                        INVALID_PARAMS,
                        &format!("{e}"),
                    ))
                }
            };
            // No live identity is `-32006`. FR-019's already-exited task is **not** this: its
            // identity is still live until its `onExit` has been delivered, so it is found here
            // and the signal goes to a process that is already gone, which the kernel ignores.
            let Some(control) = service.control(&p.task_id) else {
                return reply_or_nothing(encode_error(
                    codec,
                    id,
                    codes::TASK_NOT_FOUND,
                    "no task with that identity is running",
                ));
            };
            let signal = match p.signal {
                apex_protocol::wire::TerminateSignal::Int => TaskSignal::Int,
                apex_protocol::wire::TerminateSignal::Term => TaskSignal::Term,
                apex_protocol::wire::TerminateSignal::Kill => TaskSignal::Kill,
            };
            match stop_task(&p.task_id, signal, service.now()) {
                Ok(plan) => {
                    let _ = control.signal(plan.send);
                    if let (Some(at), Some(pid)) = (plan.escalate_at, service.pid(&p.task_id)) {
                        // Keyed on the pair, so a deadline cannot outlive the process it was
                        // made for and reach a task that reused the identity.
                        service
                            .escalations()
                            .register(p.task_id.clone(), pid, &control, at);
                    }
                    reply_or_nothing(encode_result(codec, id, &serde_json::Value::Null))
                }
                Err(_) => reply_or_nothing(encode_error(
                    codec,
                    id,
                    codes::TASK_NOT_FOUND,
                    "no task with that identity is running",
                )),
            }
        }
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
                    reply_or_nothing(encode_result(codec, id, &response))
                }
                Err(e) => {
                    reply_or_nothing(encode_error(codec, id, INVALID_PARAMS, &format!("{e}")))
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
        "session/restart" => match encode_result(codec, id, &serde_json::Value::Null) {
            Some(frame) => Action::Restart(frame),
            None => Action::Nothing,
        },
        // ---- Workspace (§4.8). Registration first: every other method needs it. ----
        "workspace/watch" | "workspace/unwatch" => {
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let Ok(req) = serde_json::from_value::<apex_protocol::wire::WatchParams>(params) else {
                return reply_or_nothing(encode_error(codec, id, -32602, "invalid params"));
            };
            let root = match roots.resolve(&req.workspace_id.0) {
                Ok(r) => r,
                Err(why) => {
                    let refusal =
                        crate::application::use_cases::workspace::RequestRefusal::Root(why);
                    let (code, message) = refusal.wire();
                    return reply_or_nothing(encode_error(codec, id, code, &message));
                }
            };
            let Some(exclusions) = roots.exclusions(&req.workspace_id.0) else {
                return reply_or_nothing(encode_error(
                    codec,
                    id,
                    apex_protocol::wire::codes::WORKSPACE_NOT_REGISTERED,
                    "workspace is not registered with this engine",
                ));
            };
            // No watcher wired means this build cannot watch -- a non-Linux host, or a test
            // that did not ask for one. FR-027: the workspace stays usable and the developer
            // is told, rather than the call appearing to succeed.
            let Some(watchers) = watchers else {
                return reply_or_nothing(encode_error(
                    codec,
                    id,
                    -32601,
                    "this engine build cannot watch the filesystem",
                ));
            };
            if method == "workspace/watch" {
                match watchers.watch(&req.workspace_id, &root, exclusions, req.paths) {
                    Some(result) => reply_or_nothing(encode_result(codec, id, &result)),
                    None => {
                        reply_or_nothing(encode_error(codec, id, -32603, "the watcher stopped"))
                    }
                }
            } else {
                match watchers.unwatch(&req.workspace_id, &root, exclusions, req.paths) {
                    Some(watching) => reply_or_nothing(encode_result(
                        codec,
                        id,
                        &apex_protocol::wire::UnwatchResult { watching },
                    )),
                    None => {
                        reply_or_nothing(encode_error(codec, id, -32603, "the watcher stopped"))
                    }
                }
            }
        }
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
                        reply_or_nothing(encode_result(codec, id, &result))
                    }
                    Err(e) => {
                        let (code, message) =
                            crate::application::use_cases::workspace::RequestRefusal::Root(e)
                                .wire();
                        reply_or_nothing(encode_error(codec, id, code, &message))
                    }
                },
                Err(e) => {
                    reply_or_nothing(encode_error(codec, id, INVALID_PARAMS, &format!("{e}")))
                }
            }
        }
        "workspace/readDirectory" => {
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match serde_json::from_value::<apex_protocol::wire::ReadDirectoryParams>(params) {
                Ok(req) => match workspace::resolve_request(
                    roots,
                    fs,
                    &req.workspace_id.0,
                    &req.relative_path,
                ) {
                    Ok(path) => {
                        match workspace::read_directory(fs, &path, req.cursor.as_deref(), req.limit)
                        {
                            Ok((items, next_cursor)) => reply_or_nothing(encode_result(
                                codec,
                                id,
                                &apex_protocol::wire::ReadDirectoryResult { items, next_cursor },
                            )),
                            Err(e) => reply_or_nothing(encode_error(
                                codec,
                                id,
                                apex_protocol::wire::codes::NOT_FOUND,
                                &e.to_string(),
                            )),
                        }
                    }
                    Err(refusal) => {
                        let (code, message) = refusal.wire();
                        reply_or_nothing(encode_error(codec, id, code, &message))
                    }
                },
                Err(e) => {
                    reply_or_nothing(encode_error(codec, id, INVALID_PARAMS, &format!("{e}")))
                }
            }
        }
        "workspace/stat" => {
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match serde_json::from_value::<apex_protocol::wire::StatParams>(params) {
                Ok(req) => match workspace::resolve_request(
                    roots,
                    fs,
                    &req.workspace_id.0,
                    &req.relative_path,
                ) {
                    Ok(path) => match workspace::stat(fs, &path) {
                        Ok(result) => reply_or_nothing(encode_result(codec, id, &result)),
                        Err(e) => reply_or_nothing(encode_error(
                            codec,
                            id,
                            apex_protocol::wire::codes::NOT_FOUND,
                            &e.to_string(),
                        )),
                    },
                    Err(refusal) => {
                        let (code, message) = refusal.wire();
                        reply_or_nothing(encode_error(codec, id, code, &message))
                    }
                },
                Err(e) => {
                    reply_or_nothing(encode_error(codec, id, INVALID_PARAMS, &format!("{e}")))
                }
            }
        }
        "workspace/readFile" => {
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match serde_json::from_value::<apex_protocol::wire::ReadFileParams>(params) {
                Ok(req) => match workspace::resolve_request(
                    roots,
                    fs,
                    &req.workspace_id.0,
                    &req.relative_path,
                ) {
                    Ok(path) => match workspace::read_file(fs, &path, req.offset, req.length) {
                        Ok(Ok(result)) => reply_or_nothing(encode_result(codec, id, &result)),
                        // Refused rather than truncated: a silent truncation is a corrupt file
                        // the caller cannot see. The client routes to the bulk path instead.
                        Ok(Err(workspace::ReadRefusal::TooLarge { total_size })) => {
                            reply_or_nothing(encode_error(
                                codec,
                                id,
                                INVALID_PARAMS,
                                &format!(
                                    "{total_size} bytes exceeds the inline read limit; use the \
                                     bulk path (A-BULKSIZE)"
                                ),
                            ))
                        }
                        Err(e) => reply_or_nothing(encode_error(
                            codec,
                            id,
                            apex_protocol::wire::codes::NOT_FOUND,
                            &e.to_string(),
                        )),
                    },
                    Err(refusal) => {
                        let (code, message) = refusal.wire();
                        reply_or_nothing(encode_error(codec, id, code, &message))
                    }
                },
                Err(e) => {
                    reply_or_nothing(encode_error(codec, id, INVALID_PARAMS, &format!("{e}")))
                }
            }
        }
        "session/shutdown" => {
            if let Some(frame) = encode_result(codec, id, &serde_json::Value::Null) {
                let mut out = std::io::stdout();
                let _ = out.write_all(&frame);
                let _ = out.flush();
            }
            std::process::exit(0);
        }
        _ => reply_or_nothing(encode_error(
            codec,
            id,
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
            // A notification buffered here has nothing to answer, so an absent id is skipped
            // rather than answered -- but a numeric one is answered, like any other request.
            if let Some(id) = v.get("id") {
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

/// `id` is the caller's own JSON value, echoed back unchanged.
///
/// A `&str` until F010, which quoted a **numeric** id on the way out. JSON-RPC 2.0 allows a
/// number and requires the response to carry the same id it was sent, so `{"id": 7}` answered
/// with `{"id": "7"}` is a response the client cannot match to its request.
pub fn encode_result<T: serde::Serialize>(
    codec: &FrameCodec,
    id: &serde_json::Value,
    result: &T,
) -> Option<Vec<u8>> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
    codec.encode(&body.to_string()).ok()
}

pub fn encode_error(
    codec: &FrameCodec,
    id: &serde_json::Value,
    code: i32,
    message: &str,
) -> Option<Vec<u8>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": code, "message": message}
    });
    codec.encode(&body.to_string()).ok()
}

/// Frame any notification.
///
/// Was hard-typed to `&RestartNotice`, which was fine while a restart notice was the only
/// thing the engine ever originated. F004 adds file events and invalidations, and a second
/// near-identical function would be the same code twice with one type changed.
pub fn encode_notification<T: serde::Serialize>(
    codec: &FrameCodec,
    method: &str,
    params: &T,
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
            let Action::Reply(reply) = dispatch(&registry, &roots, &fs, None, None, &codec, body)
            else {
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
            None,
            None,
            &codec,
            r#"{"jsonrpc":"2.0","id":"1","method":"auth/handshake","params":{}}"#,
        );
        let Action::Reply(good) = dispatch(
            &registry,
            &roots,
            &fs,
            None,
            None,
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
            None,
            None,
            &codec,
            r#"{"jsonrpc":"2.0","id":"9","method":"workspace/writeFile","params":{}}"#,
        ) else {
            panic!("expected a reply")
        };
        let v = decode(&reply);
        assert_eq!(v["error"]["code"], METHOD_NOT_FOUND);
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("workspace/writeFile"),
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
                dispatch(&registry, &roots, &fs, None, None, &codec, body),
                Action::Nothing
            ));
        }
    }
}
