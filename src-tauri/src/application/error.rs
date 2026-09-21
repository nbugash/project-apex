//! The typed error surface of the command boundary.
//! See specs/001-app-shell/contracts/shell-commands.md.

use crate::domain::layout::LayoutError;
use crate::domain::session::SessionError;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ShellError {
    #[error("unknown region identifier")]
    InvalidRegion,
    #[error("requested extent is below the minimum for a visible region")]
    ExtentBelowMinimum,
    #[error("the document area cannot be hidden")]
    DocumentAreaNotHideable,
    #[error("display name must be 1-255 characters")]
    InvalidDisplayName,
    #[error("no open document with that identifier")]
    UnknownDocument,
    #[error("target position is outside the current tab count")]
    OrderOutOfRange,
    /// Reported, but never blocks the interaction that triggered it: a failed write should
    /// degrade persistence, not usability (FR-023).
    #[error("session state could not be written")]
    PersistenceFailed,
}

impl From<LayoutError> for ShellError {
    fn from(e: LayoutError) -> Self {
        match e {
            LayoutError::ExtentBelowMinimum => Self::ExtentBelowMinimum,
            LayoutError::DocumentAreaNotHideable => Self::DocumentAreaNotHideable,
        }
    }
}

impl From<SessionError> for ShellError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::InvalidDisplayName => Self::InvalidDisplayName,
            SessionError::UnknownDocument => Self::UnknownDocument,
            SessionError::OrderOutOfRange => Self::OrderOutOfRange,
        }
    }
}
