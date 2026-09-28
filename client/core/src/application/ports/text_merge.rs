//! Three versions in, one decision out. Pure.
//!
//! A port because the decision is the feature's central property and must be replaceable without
//! touching the reconciler: SC-006b requires the client's choice to merge or prompt to match a
//! standard version-control three-way merge, and that is a claim about *this* implementation which
//! a use case holding a library directly could not make separately testable.

/// What a three-way merge concluded.
///
/// **`Conflict` carries nothing.** A partially merged file is not something this feature may
/// produce: the developer is shown three versions and chooses, and handing them a file with
/// conflict markers in it would be a fourth version nobody wrote. The reconciler needs to know
/// only whether it may write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// The two changes combine. This is the text to write.
    Clean(String),
    /// The changes genuinely collide, so the developer decides (FR-021).
    Conflict,
}

/// Decide, purely, whether three versions combine or collide.
///
/// Pure: no clock, no filesystem, no connection. That is what lets the agreement suite run twenty
/// pairs through it without a workspace, and what keeps the conflict boundary a property of one
/// function rather than of a sequence of calls.
pub trait TextMerge: Send + Sync {
    /// `base` is the content the host confirmed, `local` what the developer saved offline, `remote`
    /// what the host holds now. All three are text: the caller has already established that, because
    /// a file the client does not hold as text is never merged (FR-025a).
    fn merge(&self, base: &str, local: &str, remote: &str) -> MergeOutcome;
}
