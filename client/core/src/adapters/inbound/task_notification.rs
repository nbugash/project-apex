//! Wire payload to use-case input.
//!
//! An inbound adapter and nothing more: it translates and carries no business rule. The one thing
//! it decides is that a `data` field which is not valid base64 is dropped rather than passed on,
//! which is not a rule so much as the boundary itself.
//!
//! **The base64 is decoded exactly once, here.** Every layer above this one takes `Vec<u8>`, so
//! there is no second place where a decode could be attempted and no opportunity for one of them
//! to go through a text encoding on the way.

use apex_protocol::base64;
use apex_protocol::wire::{ExitParams, OutputParams};

use crate::application::use_cases::observe_task::{Notification, Stream};

/// An `execution/onStdout` or `execution/onStderr` payload.
///
/// `None` when `data` is not base64. A frame the client cannot decode is a frame it cannot act
/// on, and inventing bytes for it would put fabricated output in front of somebody reading a
/// build log.
pub fn translate_output(params: &OutputParams, stream: Stream) -> Option<Notification> {
    let bytes = base64::decode(&params.data).ok()?;
    Some(Notification::Output {
        task: params.task_id.clone(),
        stream,
        bytes,
    })
}

/// An `execution/onExit` payload.
///
/// Both fields are carried through as they arrived, including the impossible combinations. The
/// use case decides what they mean, because deciding is a rule and this is an adapter.
pub fn translate_exit(params: &ExitParams) -> Notification {
    Notification::Exit {
        task: params.task_id.clone(),
        exit_code: params.exit_code,
        signal: params.signal.clone(),
    }
}
