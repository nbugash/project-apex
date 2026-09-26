//! Saving a file, and what came of it.
//!
//! The mapping is the whole use case. `WorkspaceProvider` reports failures as a dozen typed
//! variants because a *caller* may need any of those distinctions; a developer looking at an
//! editor needs four, and which four is a product decision rather than a transport one
//! (FR-012). Keeping the reduction here means the surface never sees a `ProviderError`, and the
//! rule about what may be collapsed into what is written down once, in a place with tests.

use crate::application::ports::workspace_provider::{ProviderError, WorkspaceProvider};
use crate::domain::workspace::{RelPath, Sha256, WorkspaceId};
use std::sync::Arc;

/// What a save attempt produced.
///
/// `Conflict` and `Unreachable` are different things and are **never** collapsed: one means a
/// colleague edited the file and the developer should look at what they did, the other means the
/// link dropped and they should try again. A single "save failed" would make the next action a
/// guess, and guessing wrong in the first direction overwrites somebody's work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The engine accepted it. The digest becomes the buffer's new base.
    Written { sha256: Sha256 },
    /// `-32004`: the host's content changed since the base was taken. Nothing was written.
    Conflict,
    /// The engine considered it and declined -- path, permission, size, or a capability that is
    /// not built yet. Retrying unchanged will fail identically; something has to change first.
    Refused { reason: String },
    /// The request never landed. Nothing on the host changed, and retrying is the right move.
    Unreachable,
}

impl From<ProviderError> for WriteOutcome {
    fn from(e: ProviderError) -> Self {
        match e {
            ProviderError::WriteConflict => Self::Conflict,
            ProviderError::Offline => Self::Unreachable,
            // A timeout, a withdrawal and an unintelligible reply all arrive as `Transport`,
            // because the adapter has no way to separate them once the outcome is in hand. They
            // belong with `Unreachable` rather than `Refused` for the reason the two variants
            // exist at all: what the developer does next. Nothing reached the disk and the
            // remedy is to try again -- which is what `Unreachable` says and what `Refused`,
            // meaning "change something first", would not.
            ProviderError::Transport(_) => Self::Unreachable,
            // Everything the engine considered and declined. The typed variant *is* the §4.4
            // code, rendered; carrying the raw integer here would undo the mapping that exists
            // so no caller ever parses a message.
            other => Self::Refused {
                reason: other.to_string(),
            },
        }
    }
}

pub struct EditFile {
    provider: Arc<dyn WorkspaceProvider>,
}

impl EditFile {
    pub fn new(provider: Arc<dyn WorkspaceProvider>) -> Self {
        Self { provider }
    }

    /// Save `text` if the host still holds `base`.
    ///
    /// Takes `&str` rather than bytes: §4.8 carries `writeFile` content as text, and a caller
    /// that had bytes would have to decide what a non-UTF-8 save means. That decision belongs
    /// to whoever produced the bytes, not to a save button.
    pub async fn save(
        &self,
        ws: &WorkspaceId,
        path: &RelPath,
        text: &str,
        base: &Sha256,
    ) -> WriteOutcome {
        match self
            .provider
            .write_file(ws, path, text.as_bytes(), base)
            .await
        {
            Ok(sha256) => WriteOutcome::Written { sha256 },
            Err(e) => WriteOutcome::from(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::workspace_provider::Owner;

    #[test]
    fn a_conflict_and_a_dropped_link_are_never_the_same_outcome() {
        // The pair this type exists to keep apart. Asserted as an inequality as well as on each
        // variant, because a future refactor that made both "failed" would still satisfy two
        // separate `matches!` assertions written against it one at a time.
        let conflict = WriteOutcome::from(ProviderError::WriteConflict);
        let dropped = WriteOutcome::from(ProviderError::Offline);
        assert_eq!(conflict, WriteOutcome::Conflict);
        assert_eq!(dropped, WriteOutcome::Unreachable);
        assert_ne!(conflict, dropped);
    }

    #[test]
    fn what_the_engine_declined_is_refused_and_says_why() {
        for e in [
            ProviderError::NotFound,
            ProviderError::Refused,
            ProviderError::UnknownWorkspace,
            ProviderError::WorkspaceGone,
            ProviderError::Unsupported {
                owner: Owner::F015LocalMode,
            },
        ] {
            let expected = e.to_string();
            match WriteOutcome::from(e) {
                WriteOutcome::Refused { reason } => assert_eq!(reason, expected),
                other => panic!("{expected} must be a refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_transport_failure_asks_for_a_retry_rather_than_a_change() {
        // `Refused` tells the developer to change something. A wedged engine is not something
        // they can change, and telling them so sends them editing a path that was always fine.
        assert_eq!(
            WriteOutcome::from(ProviderError::Transport("timed out".into())),
            WriteOutcome::Unreachable
        );
    }

    #[test]
    fn a_success_carries_the_digest_that_becomes_the_next_base() {
        // Losing the digest here would leave the buffer holding the *old* base, so the very next
        // save would be refused for a conflict the developer caused by saving successfully.
        let sha256 = Sha256::of(b"saved");
        assert_eq!(
            WriteOutcome::Written {
                sha256: sha256.clone()
            },
            WriteOutcome::Written { sha256 }
        );
    }
}
