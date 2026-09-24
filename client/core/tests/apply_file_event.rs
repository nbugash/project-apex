//! What the client does with a batch it was told about (FR-014, FR-019, FR-020, FR-023b).

mod common;

use apex_shell::adapters::inbound::file_event_notification::translate;
use apex_shell::application::use_cases::apply_file_event::{ApplyFileEvent, Change, FileEvent};
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

fn rel(p: &str) -> RelPath {
    RelPath::parse(p).expect("a path")
}

#[test]
fn a_created_event_is_applied_without_any_further_request() {
    // US1 acceptance 1 and FR-020. The metadata the event carries is what makes this possible;
    // the alternative was asking about a path the client was just told about.
    let cache = InMemoryCache::default();
    let apply = ApplyFileEvent::new(&cache);
    let report = apply.apply(
        &ws(),
        &[FileEvent {
            path: rel("src/token.rs"),
            change: Change::Created {
                is_directory: false,
                size: 42,
                modified: 7,
            },
        }],
    );
    assert_eq!(report.applied, 1);
    assert_eq!(report.refused, 0);
}

#[test]
fn an_event_never_marks_content_valid() {
    // FR-019. Structural: there is no method on this use case that could.
    let cache = InMemoryCache::default();
    let apply = ApplyFileEvent::new(&cache);
    let report = apply.apply(
        &ws(),
        &[FileEvent {
            path: rel("a.rs"),
            change: Change::Modified {
                is_directory: false,
                size: 1,
                modified: 1,
            },
        }],
    );
    assert_eq!(report.marked_unproven, 1, "doubt is all an event can cast");
}

#[test]
fn a_path_that_does_not_parse_is_refused_at_this_end() {
    // FR-014 and SC-010. Independently of anything the engine checked: a boundary enforced on
    // one side only is a boundary enforced nowhere.
    use apex_protocol::wire::{FileEvent as WireEvent, FileEventKind};
    let translated = translate(&[
        WireEvent {
            event: FileEventKind::Modified,
            relative_path: "../../etc/shadow".into(),
            to_path: None,
            kind: None,
            size: None,
            modified: None,
        },
        WireEvent {
            event: FileEventKind::Modified,
            relative_path: "src/ok.rs".into(),
            to_path: None,
            kind: None,
            size: None,
            modified: None,
        },
    ]);
    assert_eq!(translated.refused, 1, "the escaping path was refused");
    assert_eq!(
        translated.events.len(),
        1,
        "the contained one still arrived"
    );
    assert_eq!(
        translated.events[0].path.as_str(),
        "/src/ok.rs",
        "RelPath normalises with a leading slash"
    );
}

#[test]
fn a_renames_destination_is_untrusted_too() {
    use apex_protocol::wire::{FileEvent as WireEvent, FileEventKind};
    let translated = translate(&[WireEvent {
        event: FileEventKind::Renamed,
        relative_path: "src/a.rs".into(),
        to_path: Some("../../elsewhere".into()),
        kind: None,
        size: None,
        modified: None,
    }]);
    assert_eq!(
        translated.refused, 1,
        "a rename carries two paths and both are untrusted"
    );
    assert!(translated.events.is_empty());
}

#[test]
fn a_wholesale_invalidation_marks_every_open_tab_unproven() {
    // FR-023b and SC-004a. The bulk rule discards the individual events, so without this a
    // branch switch that rewrote a file the developer has open reports nothing about it.
    let cache = InMemoryCache::default();
    let apply = ApplyFileEvent::new(&cache);
    let tabs = [rel("src/main.rs"), rel("src/lib.rs"), rel("README.md")];
    let report = apply.invalidate_all(&ws(), &tabs);
    assert_eq!(
        report.marked_unproven, 3,
        "every open tab, not only the focused one -- FR-023 admits no exception for how a \
         change arrived"
    );
    assert_eq!(report.applied, 1, "the tree was marked stale exactly once");

    // And nothing else: a file with no open tab must not be marked, or FR-024 is breached by
    // the mechanism meant to satisfy FR-023.
    let none = apply.invalidate_all(&ws(), &[]);
    assert_eq!(none.marked_unproven, 0);
}

#[test]
fn an_empty_batch_does_nothing_rather_than_failing() {
    let cache = InMemoryCache::default();
    let apply = ApplyFileEvent::new(&cache);
    assert_eq!(apply.apply(&ws(), &[]).applied, 0);
}
