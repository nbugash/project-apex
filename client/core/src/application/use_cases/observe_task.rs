//! What the client does with output and endings it was told about.
//!
//! Two rules govern everything here.
//!
//! **Output bytes are never decoded into a `String`.** A task's output is bytes: a compiler emits
//! them in whatever encoding it pleases, and a program under test emits whatever it was asked to.
//! A decode at this layer substitutes U+FFFD for anything that is not valid UTF-8 and the loss is
//! invisible at the engine, which sent the bytes intact. So they travel as `Vec<u8>` from the
//! adapter that decoded the base64 all the way to the panel, and nothing in between looks at them.
//!
//! **An ending is read, not guessed.** §4.8 carries `exitCode` **or** `signal` and never both.
//! A frame with both or neither is a protocol error, and it is surfaced as one rather than
//! resolved into whichever field happens to be present -- a client that reads `exitCode` from a
//! signalled death gets `null` or `0` depending on the serialiser, and both read as success.
//! Re-checked here although the engine also checks it, because a boundary enforced on one side
//! only is a boundary enforced nowhere (Principle VI).

use std::collections::BTreeSet;

use apex_protocol::wire::{SignalName, TaskId};

/// Which of a task's two streams a chunk came from.
///
/// The client's own type rather than the engine's. The wire does not carry a stream *field* -- it
/// carries two method names, `execution/onStdout` and `execution/onStderr` -- so this is derived
/// at the boundary from which method arrived, and reaching into the engine's domain for a shared
/// enum would couple the two halves for a value neither actually sends.
///
/// A task given a terminal produces only `Stdout`, because a terminal is one device and both
/// streams are already merged onto it before the engine sees them (A-TASKSTREAM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// How a task finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// The process exited of its own accord, with this status.
    Code(i32),
    /// The process was killed. A **name**, never `128 + n`: the wire spent a field preserving
    /// the distinction and reconstructing the convention here would throw it away.
    Signal(SignalName),
    /// Both fields, or neither. Reported rather than resolved.
    Unintelligible,
}

/// What a panel is handed. The use case's output, so a caller asserts on a value rather than on
/// a side effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelInput {
    Output {
        task: TaskId,
        stream: Stream,
        bytes: Vec<u8>,
    },
    Ended {
        task: TaskId,
        ending: Ending,
    },
}

/// One notification, after the wire types have been translated away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    Output {
        task: TaskId,
        stream: Stream,
        bytes: Vec<u8>,
    },
    Exit {
        task: TaskId,
        exit_code: Option<i32>,
        signal: Option<SignalName>,
    },
}

/// Why a notification produced nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// For a task this client is not watching.
    ///
    /// Not an error. A-TASKLIFE lets a task outlive the connection that started it, and
    /// `execution/list` lets a client discover tasks it never started, so output for an unknown
    /// identity is an ordinary consequence of a shared engine rather than a fault.
    NotWatched,
}

#[derive(Debug, Default)]
pub struct ObserveTask {
    watching: BTreeSet<TaskId>,
}

impl ObserveTask {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin watching a task, whether this client started it or attached to it.
    pub fn watch(&mut self, task: TaskId) {
        self.watching.insert(task);
    }

    pub fn is_watching(&self, task: &TaskId) -> bool {
        self.watching.contains(task)
    }

    /// Route one notification to a panel.
    ///
    /// An exit stops the watch, which is what keeps the set from growing for the life of the
    /// session. It stops it **after** producing the panel input, so the ending is delivered --
    /// releasing first would drop the very frame that says the task is over.
    pub fn observe(&mut self, notification: Notification) -> Result<PanelInput, Ignored> {
        match notification {
            Notification::Output {
                task,
                stream,
                bytes,
            } => {
                if !self.watching.contains(&task) {
                    return Err(Ignored::NotWatched);
                }
                Ok(PanelInput::Output {
                    task,
                    stream,
                    bytes,
                })
            }
            Notification::Exit {
                task,
                exit_code,
                signal,
            } => {
                if !self.watching.contains(&task) {
                    return Err(Ignored::NotWatched);
                }
                let ending = match (exit_code, signal) {
                    (Some(code), None) => Ending::Code(code),
                    (None, Some(name)) => Ending::Signal(name),
                    _ => Ending::Unintelligible,
                };
                self.watching.remove(&task);
                Ok(PanelInput::Ended { task, ending })
            }
        }
    }
}
