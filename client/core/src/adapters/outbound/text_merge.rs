//! The three-way merge, and the **only** file in this crate that names `diffy`.
//!
//! `tests/merge_confinement.rs` enforces that. The rule is not tidiness: if the merge library may
//! be named anywhere then anywhere may decide what a conflict is, and the conflict boundary is the
//! property SC-006b pins down. The same rule and the same reason as
//! `engine/tests/inotify_confinement.rs`.
//!
//! Why `diffy` at all is recorded in specs/010-offline-editing/research.md, and it was **measured**
//! rather than argued: eight cases run through both `diffy::merge` and `git merge-file`, zero
//! disagreements, including the two that decide the design -- adjacent lines must conflict and two
//! lines apart must merge cleanly. No link here, because no Rust source file in this repository
//! names a URL and `make no-network` enforces it.

use crate::application::ports::text_merge::{MergeOutcome, TextMerge};

/// A three-way merge with git's own conflict boundary.
#[derive(Debug, Default, Clone, Copy)]
pub struct DiffyMerge;

impl DiffyMerge {
    pub fn new() -> Self {
        Self
    }
}

impl TextMerge for DiffyMerge {
    fn merge(&self, base: &str, local: &str, remote: &str) -> MergeOutcome {
        // `local` is ours and `remote` is theirs. The order matters for which side a clean merge
        // prefers when both changed the same line identically, and it matters for nothing else --
        // a genuine collision conflicts either way, which is the property the agreement suite
        // measures rather than assumes.
        match diffy::merge(base, local, remote) {
            Ok(merged) => MergeOutcome::Clean(merged),
            // The error carries the merged text *with conflict markers in it*, and it is discarded
            // deliberately. Handing that to the developer would be a fourth version nobody wrote,
            // and FR-021 says the client prompts rather than writing.
            Err(_) => MergeOutcome::Conflict,
        }
    }
}
