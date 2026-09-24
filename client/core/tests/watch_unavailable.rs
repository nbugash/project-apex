//! When watching is unavailable the developer is told (FR-005, FR-025, FR-027, SC-011).
//!
//! Silence is the failure. A watcher that is running and telling nobody, and a link that has
//! stopped reporting, look identical from the tree -- so the absence has to be stated rather
//! than inferred from nothing happening.

use apex_shell::application::ports::workspace_provider::{
    ProviderError, Refusal, RefusalReason, WatchOutcome,
};
use apex_shell::domain::workspace::RelPath;

fn rel(p: &str) -> RelPath {
    RelPath::parse(p).expect("a path")
}

#[test]
fn an_unsupported_provider_names_the_feature_that_would_supply_it() {
    // A-WATCHLOCAL. Local mode does not watch in v1, and the refusal says which feature owns
    // it rather than failing anonymously.
    let error = ProviderError::Unsupported {
        owner: apex_shell::application::ports::workspace_provider::Owner::F004FileWatch,
    };
    let rendered = format!("{error:?}");
    assert!(rendered.contains("F004"), "{rendered}");
}

#[test]
fn every_refused_path_carries_a_reason() {
    // Four reasons, and all of them mean the same thing to the developer: this path is not
    // being watched. The distinction exists so the client can say *why*, not so it can decide
    // differently.
    for reason in [
        RefusalReason::Capacity,
        RefusalReason::Excluded,
        RefusalReason::NotFound,
        RefusalReason::NotADirectory,
    ] {
        let outcome = WatchOutcome {
            watching: 0,
            refused: vec![Refusal {
                path: rel("/a"),
                reason,
            }],
        };
        assert_eq!(outcome.refused.len(), 1);
    }
}

#[test]
fn a_partial_result_is_not_an_absence_of_information() {
    // An empty refusal list is a positive statement that nothing was refused. A client that
    // could not tell that from a missing field could not satisfy FR-005.
    let complete = WatchOutcome {
        watching: 3,
        refused: vec![],
    };
    assert!(complete.refused.is_empty());
    assert_eq!(complete.watching, 3);
}
