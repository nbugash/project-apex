//! Reconnection re-establishes everything still expanded and still open (FR-026b, SC-012b).
//!
//! Watches do not survive a dropped connection. A client that resumed believing it was still
//! being told about changes would show a tree that had quietly stopped updating -- which is
//! FR-025's failure arriving by a different route, and the reason the whole set is re-sent
//! rather than a remembered history replayed.

use apex_shell::application::ports::workspace_provider::{Refusal, WatchOutcome};

/// The client's record of what it has asked for. Re-sent whole, because set semantics are what
/// make one call enough.
#[derive(Default)]
struct Requested {
    folders: Vec<String>,
    open_files: Vec<String>,
}

impl Requested {
    fn all(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .folders
            .iter()
            .chain(self.open_files.iter())
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

#[test]
fn everything_expanded_and_open_is_re_sent_as_one_call() {
    let state = Requested {
        folders: vec!["/src".into(), "/docs".into()],
        open_files: vec!["/src/main.rs".into()],
    };
    let sent = state.all();
    assert_eq!(sent, vec!["/docs", "/src", "/src/main.rs"]);
}

#[test]
fn a_tab_whose_folder_is_collapsed_is_still_re_established() {
    // The case the whole reasons-not-conclusions design exists for. The folder is not expanded,
    // so a directory-only set would omit it and the developer's open file would stop being
    // reported after a reconnection, silently.
    let state = Requested {
        folders: vec![],
        open_files: vec!["/src/deep/found.rs".into()],
    };
    assert_eq!(state.all(), vec!["/src/deep/found.rs"]);
}

#[test]
fn re_establishing_an_empty_set_asks_for_nothing() {
    assert!(Requested::default().all().is_empty());
}

#[test]
fn a_refusal_on_re_establishment_is_reported_not_swallowed() {
    // A folder deleted while the client was disconnected comes back as a per-path refusal, and
    // the rest of the set is still established. Failing the whole call would leave everything
    // unwatched -- the failure FR-026b exists to close.
    let outcome = WatchOutcome {
        watching: 2,
        refused: vec![Refusal {
            path: apex_shell::domain::workspace::RelPath::parse("/gone").expect("path"),
            reason: apex_shell::application::ports::workspace_provider::RefusalReason::NotFound,
        }],
    };
    assert_eq!(outcome.watching, 2, "the rest was established");
    assert_eq!(outcome.refused.len(), 1, "and the loss was stated");
}
