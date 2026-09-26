//! A chunk becomes panel input, an exit becomes panel input, and the bytes survive intact.
//!
//! SC-003's client half. The engine's half proves the bytes left it unchanged; this proves they
//! are still unchanged after the client's inbound adapter has handled them, which is the other
//! place a decode to text could happen and the place where it would be invisible from the engine.

use apex_protocol::wire::{ExitParams, OutputParams, SignalName, TaskId};
use apex_shell::adapters::inbound::task_notification::{translate_exit, translate_output};
use apex_shell::application::use_cases::observe_task::{
    Ending, Ignored, Notification, ObserveTask, PanelInput, Stream,
};

/// Four different ways to be invalid UTF-8, so a decoder that repairs one still fails here.
const NOT_UTF8: &[u8] = &[
    0x80, 0xED, 0xA0, 0x80, 0xC0, 0xAF, 0xFF, 0xFE, 0x00, 0x01, 0x7F,
];

/// U+FFFD, encoded. Its presence is the symptom.
const REPLACEMENT: &[u8] = &[0xEF, 0xBF, 0xBD];

fn task() -> TaskId {
    TaskId("build".into())
}

fn watching() -> ObserveTask {
    let mut o = ObserveTask::new();
    o.watch(task());
    o
}

fn output_params(bytes: &[u8]) -> OutputParams {
    OutputParams {
        task_id: task(),
        data: apex_protocol::base64::encode(bytes),
    }
}

#[test]
fn a_chunk_becomes_panel_input() {
    let mut observe = watching();
    let n = translate_output(&output_params(b"compiling"), Stream::Stdout).expect("decodes");
    let input = observe.observe(n).expect("watched");
    assert_eq!(
        input,
        PanelInput::Output {
            task: task(),
            stream: Stream::Stdout,
            bytes: b"compiling".to_vec(),
        }
    );
}

#[test]
fn the_bytes_survive_the_inbound_adapter_unchanged() {
    // The assertion this file exists for. A decode to String anywhere on this path substitutes
    // U+FFFD, and the engine cannot tell -- it sent the bytes intact.
    let mut observe = watching();
    let n = translate_output(&output_params(NOT_UTF8), Stream::Stdout).expect("decodes");
    let PanelInput::Output { bytes, .. } = observe.observe(n).expect("watched") else {
        panic!("a chunk must become output");
    };
    assert_eq!(bytes, NOT_UTF8, "the bytes changed crossing the boundary");
    assert_eq!(
        bytes
            .windows(REPLACEMENT.len())
            .filter(|w| *w == REPLACEMENT)
            .count(),
        0,
        "U+FFFD appears, so something decoded the output as text"
    );
}

#[test]
fn every_byte_value_survives() {
    // 0..255 with no gaps. A path that mangles only the high half would pass a test using
    // printable ASCII, and most of a build's output is printable ASCII.
    let all: Vec<u8> = (0..=255u8).collect();
    let mut observe = watching();
    let n = translate_output(&output_params(&all), Stream::Stderr).expect("decodes");
    let PanelInput::Output { bytes, stream, .. } = observe.observe(n).expect("watched") else {
        panic!("a chunk must become output");
    };
    assert_eq!(bytes, all);
    assert_eq!(stream, Stream::Stderr, "the stream came from the method");
}

#[test]
fn an_exit_becomes_panel_input_carrying_a_code() {
    let mut observe = watching();
    let n = translate_exit(&ExitParams {
        task_id: task(),
        exit_code: Some(7),
        signal: None,
    });
    assert_eq!(
        observe.observe(n).expect("watched"),
        PanelInput::Ended {
            task: task(),
            ending: Ending::Code(7),
        }
    );
}

#[test]
fn a_signalled_exit_keeps_the_name_rather_than_becoming_a_number() {
    // The wire spends a field preserving the distinction. Reconstructing 128 + n here would
    // throw it away, and 128 + n is indistinguishable from a program that exited with that code.
    let mut observe = watching();
    let n = translate_exit(&ExitParams {
        task_id: task(),
        exit_code: None,
        signal: Some(SignalName("SIGKILL".into())),
    });
    assert_eq!(
        observe.observe(n).expect("watched"),
        PanelInput::Ended {
            task: task(),
            ending: Ending::Signal(SignalName("SIGKILL".into())),
        }
    );
}

#[test]
fn an_exit_carrying_both_fields_or_neither_is_reported_rather_than_guessed() {
    // §4.8 carries one or the other and never both. A client that picks whichever is present
    // reads `exitCode` from a signalled death as 0 or null, and both read as success.
    for (code, signal) in [(Some(0), Some(SignalName("SIGKILL".into()))), (None, None)] {
        let mut observe = watching();
        let n = translate_exit(&ExitParams {
            task_id: task(),
            exit_code: code,
            signal,
        });
        assert_eq!(
            observe.observe(n).expect("watched"),
            PanelInput::Ended {
                task: task(),
                ending: Ending::Unintelligible,
            }
        );
    }
}

#[test]
fn output_for_a_task_this_client_is_not_watching_is_ignored() {
    // Not an error. A task outlives the connection that started it and `execution/list` reveals
    // tasks this client never started, so an unknown identity is ordinary.
    let mut observe = ObserveTask::new();
    let n = translate_output(&output_params(b"not mine"), Stream::Stdout).expect("decodes");
    assert_eq!(observe.observe(n), Err(Ignored::NotWatched));
}

#[test]
fn an_exit_stops_the_watch_but_is_itself_delivered() {
    // Releasing before producing the input would drop the very frame that says the task is over.
    let mut observe = watching();
    assert!(observe.is_watching(&task()));
    let n = translate_exit(&ExitParams {
        task_id: task(),
        exit_code: Some(0),
        signal: None,
    });
    assert!(observe.observe(n).is_ok(), "the ending must be delivered");
    assert!(!observe.is_watching(&task()), "the watch must then stop");

    let later = translate_output(&output_params(b"after"), Stream::Stdout).expect("decodes");
    assert_eq!(observe.observe(later), Err(Ignored::NotWatched));
}

#[test]
fn data_that_is_not_base64_is_dropped_rather_than_invented() {
    let params = OutputParams {
        task_id: task(),
        data: "not valid base64!!".into(),
    };
    assert!(translate_output(&params, Stream::Stdout).is_none());
}

#[test]
fn a_notification_built_by_hand_still_routes() {
    // The use case takes its own input type, so it is testable without the adapter -- which is
    // what makes the adapter replaceable rather than load-bearing.
    let mut observe = watching();
    let input = observe
        .observe(Notification::Output {
            task: task(),
            stream: Stream::Stdout,
            bytes: vec![1, 2, 3],
        })
        .expect("watched");
    assert_eq!(
        input,
        PanelInput::Output {
            task: task(),
            stream: Stream::Stdout,
            bytes: vec![1, 2, 3],
        }
    );
}
