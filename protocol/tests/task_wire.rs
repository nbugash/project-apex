//! The execution vocabulary on the wire (F010).
//!
//! These assert the *frame*, not the type. A round trip through `serde` proves the types agree
//! with themselves, which they would even if every field were spelled wrong -- so each test here
//! reads the JSON text, because the JSON text is the contract a client on another machine
//! implements against.

use apex_protocol::wire::*;
use serde_json::{json, Value};

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("serialise")
}

#[test]
fn run_task_params_are_snake_case_on_the_wire() {
    let p = RunTaskParams {
        workspace_id: WorkspaceId("ws-1".into()),
        task_id: TaskId("build".into()),
        command: vec!["cargo".into(), "test".into()],
        cwd: Some("crates/engine".into()),
        env: Some([("RUST_LOG".to_string(), "debug".to_string())].into()),
        pty: true,
        cols: Some(120),
        rows: Some(40),
    };
    let v = to_value(&p);
    for key in [
        "workspace_id",
        "task_id",
        "command",
        "cwd",
        "env",
        "pty",
        "cols",
        "rows",
    ] {
        assert!(v.get(key).is_some(), "missing {key} in {v}");
    }
    // camelCase would be the natural mistake, since §4.8's tables are written that way.
    assert!(v.get("workspaceId").is_none(), "{v}");
    assert_eq!(v["command"], json!(["cargo", "test"]));
}

#[test]
fn absent_optionals_are_omitted_rather_than_null() {
    let p = RunTaskParams {
        workspace_id: WorkspaceId("ws-1".into()),
        task_id: TaskId("build".into()),
        command: vec!["make".into()],
        cwd: None,
        env: None,
        pty: false,
        cols: None,
        rows: None,
    };
    let text = serde_json::to_string(&p).expect("serialise");
    // An absent `cwd` means "the workspace root" and an absent `env` means "inherit". A null
    // would be a third thing, and nothing in §4.8 says what it means.
    for key in ["cwd", "env", "cols", "rows"] {
        assert!(!text.contains(key), "{key} should be absent: {text}");
    }
}

#[test]
fn terminate_signals_carry_their_sig_prefix() {
    // `rename_all = "UPPERCASE"` on these variants would emit "INT", "TERM", "KILL" -- which
    // round-trips perfectly and is wrong on the wire. This is why each variant is renamed.
    assert_eq!(to_value(&TerminateSignal::Int), json!("SIGINT"));
    assert_eq!(to_value(&TerminateSignal::Term), json!("SIGTERM"));
    assert_eq!(to_value(&TerminateSignal::Kill), json!("SIGKILL"));

    let back: TerminateSignal = serde_json::from_value(json!("SIGTERM")).expect("deserialise");
    assert_eq!(back, TerminateSignal::Term);
}

#[test]
fn an_unknown_signal_number_still_has_a_name() {
    assert_eq!(SignalName::from_number(9).as_str(), "SIGKILL");
    assert_eq!(SignalName::from_number(11).as_str(), "SIGSEGV");
    // Total over the host's signals: a real-time signal has no constant here, and FR-021's
    // "100% of exercised cases" does not exempt the unfamiliar.
    assert_eq!(SignalName::from_number(37).as_str(), "SIG37");
    assert_eq!(SignalName::from_number(-1).as_str(), "SIG-1");
}

#[test]
fn an_exit_carries_a_code_or_a_signal_and_the_other_is_absent() {
    let by_code = ExitParams {
        task_id: TaskId("build".into()),
        exit_code: Some(101),
        signal: None,
    };
    let v = to_value(&by_code);
    assert_eq!(v["exit_code"], json!(101));
    assert!(v.get("signal").is_none(), "{v}");

    let by_signal = ExitParams {
        task_id: TaskId("build".into()),
        exit_code: None,
        signal: Some(SignalName::from_number(15)),
    };
    let v = to_value(&by_signal);
    assert_eq!(v["signal"], json!("SIGTERM"));
    // Not `128 + n`. A client reading an exit code here would report 143 for a stop the
    // developer asked for, which is the convention Key Entities rejects.
    assert!(v.get("exit_code").is_none(), "{v}");
}

#[test]
fn a_listing_never_carries_the_environment() {
    let s = TaskSummary {
        task_id: TaskId("build".into()),
        workspace_id: WorkspaceId("ws-1".into()),
        command: vec!["cargo".into(), "test".into()],
        pty: true,
        pid: Pid(4242),
        running: true,
        exit_code: None,
        signal: None,
    };
    let text = serde_json::to_string(&s).expect("serialise");
    // FR-005a keeps a task's environment out of anything that can be read back, and a listing
    // is exactly that. There is no field for it; this asserts nobody adds one.
    assert!(!text.contains("env"), "{text}");
}

#[test]
fn output_and_input_carry_base64_rather_than_text() {
    // 0x80 is a lone continuation byte: not valid UTF-8, and unrepresentable in a JSON string.
    let raw: &[u8] = &[0x80, 0xFF, b'o', b'k'];
    let encoded = apex_protocol::base64::encode(raw);
    let o = OutputParams {
        task_id: TaskId("build".into()),
        data: encoded.clone(),
    };
    let v = to_value(&o);
    assert_eq!(v["data"], json!(encoded));

    let decoded =
        apex_protocol::base64::decode(v["data"].as_str().expect("string")).expect("decode");
    assert_eq!(decoded, raw, "the bytes must survive the wire unchanged");
}

#[test]
fn every_execution_type_round_trips() {
    let attach = AttachResult {
        pid: Pid(7),
        running: false,
        retained: 4096,
        exit_code: None,
        signal: Some(SignalName("SIGSEGV".into())),
    };
    let back: AttachResult =
        serde_json::from_value(to_value(&attach)).expect("round trip AttachResult");
    assert_eq!(back.retained, 4096);
    assert_eq!(back.signal.expect("signal").as_str(), "SIGSEGV");

    let list = ListParams { workspace_id: None };
    let text = serde_json::to_string(&list).expect("serialise");
    assert_eq!(text, "{}", "an omitted workspace means every task");
}
