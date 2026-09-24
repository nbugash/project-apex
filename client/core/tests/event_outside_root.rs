//! The client refuses an escaping path independently of the engine (FR-014, SC-010, Principle VI).

use apex_protocol::wire::{FileEvent, FileEventKind};
use apex_shell::adapters::inbound::file_event_notification::translate;

fn event(path: &str) -> FileEvent {
    FileEvent {
        event: FileEventKind::Modified,
        relative_path: path.into(),
        to_path: None,
        kind: None,
        size: None,
        modified: None,
    }
}

#[test]
fn every_escaping_shape_is_refused() {
    for hostile in [
        "../../etc/shadow",
        "src/../../outside",
        "..",
        "src/\0/x",
        "..\\..\\windows",
    ] {
        let out = translate(&[event(hostile)]);
        assert_eq!(out.refused, 1, "{hostile} was accepted");
        assert!(out.events.is_empty(), "{hostile} produced an event");
    }
}

#[test]
fn a_refusal_writes_nothing_at_all() {
    // Not "writes something harmless": the translation never produces an event, so there is
    // nothing downstream that could act on it.
    let out = translate(&[event("../../etc/shadow")]);
    assert!(out.events.is_empty());
}

#[test]
fn a_leading_slash_is_not_an_escape() {
    // `/etc/shadow` off this wire means `<workspace>/etc/shadow`, not the host's file. A
    // `RelPath` is workspace-relative and normalises to a leading slash, so refusing this
    // shape would refuse the normal form of every path the engine sends.
    let out = translate(&[event("/etc/shadow")]);
    assert_eq!(out.refused, 0);
    assert_eq!(out.events[0].path.as_str(), "/etc/shadow");
}

#[test]
fn a_contained_path_is_still_accepted() {
    // Without this the suite would pass if translation refused everything.
    let out = translate(&[event("src/main.rs")]);
    assert_eq!(out.refused, 0);
    assert_eq!(out.events.len(), 1);
}

#[test]
fn one_hostile_path_does_not_discard_the_batch() {
    // A batch is a flush, not a transaction. Dropping the honest events because one was
    // malformed would let a single bad path suppress a whole burst.
    let out = translate(&[event("../../etc/shadow"), event("src/ok.rs"), event("..")]);
    assert_eq!(out.refused, 2);
    assert_eq!(out.events.len(), 1);
}
