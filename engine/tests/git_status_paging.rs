//! Serving a status too large for one frame.
//!
//! §4.1 caps a frame at 1 MiB, and a repository mid-rebase or freshly reformatted reports far
//! more paths than that. An unbounded list is refused at the codec, so the failure without
//! paging is not a truncated tree but **no status at all** — the tree shows nothing changed,
//! which is the most misleading answer this feature could give (A-GITPAGE).

use apex_engine::application::ports::git::StatusSnapshot;
use apex_engine::application::use_cases::git_status::{PageRefusal, StatusPager};
use apex_protocol::wire::{BranchPosition, GitChange, GitStatusKind, MAX_GIT_STATUS_PAGE};

fn snapshot(n: usize) -> StatusSnapshot {
    StatusSnapshot {
        branch: BranchPosition::Branch("main".into()),
        changes: (0..n)
            .map(|i| GitChange {
                path: format!("/f{i:06}.rs"),
                status: GitStatusKind::Modified,
            })
            .collect(),
    }
}

#[test]
fn five_thousand_changes_all_arrive() {
    // SC-012. Every path marked, none lost beyond the first page.
    let pager = StatusPager::new();
    pager.hold("w1", snapshot(5_000), MAX_GIT_STATUS_PAGE);

    let mut paths = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let p = pager.page("w1", cursor.as_deref(), MAX_GIT_STATUS_PAGE).expect("page");
        assert!(
            p.changes.len() as u32 <= MAX_GIT_STATUS_PAGE,
            "a page exceeded the cap: {}",
            p.changes.len()
        );
        paths.extend(p.changes.iter().map(|c| c.path.clone()));
        match p.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(paths.len(), 5_000, "every changed path must arrive");
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), 5_000, "no path arrived twice");
}

#[test]
fn only_the_final_page_carries_no_cursor() {
    // The guarantee the client's accumulation rests on. A cursor on the last page would leave
    // it accumulating forever; its absence on an earlier one would make it commit early.
    let pager = StatusPager::new();
    pager.hold("w1", snapshot(2_500), 1_000);

    let mut cursor: Option<String> = None;
    let mut seen = 0usize;
    loop {
        let p = pager.page("w1", cursor.as_deref(), 1_000).expect("page");
        seen += p.changes.len();
        match p.next_cursor {
            Some(c) => {
                assert!(seen < 2_500, "a cursor was given after the last change");
                cursor = Some(c);
            }
            None => {
                assert_eq!(seen, 2_500, "the page with no cursor must be the last");
                break;
            }
        }
    }
}

#[test]
fn a_status_that_fits_is_one_page_with_no_cursor() {
    let pager = StatusPager::new();
    let first = pager.hold("w1", snapshot(3), MAX_GIT_STATUS_PAGE);
    assert_eq!(first.changes.len(), 3);
    assert!(first.next_cursor.is_none());
}

#[test]
fn a_clean_repository_is_an_empty_page_and_not_an_error() {
    let pager = StatusPager::new();
    let first = pager.hold("w1", snapshot(0), MAX_GIT_STATUS_PAGE);
    assert!(first.changes.is_empty());
    assert!(first.next_cursor.is_none(), "an empty final page carries no cursor");
}

#[test]
fn pages_describe_one_snapshot_even_when_the_repository_moves() {
    // The guarantee a per-page re-run of git would break invisibly: the later pages would
    // describe a repository the earlier ones did not, and the client would assemble one picture
    // out of two.
    let pager = StatusPager::new();
    let first = pager.hold("w1", snapshot(2_000), 1_000);
    let cursor = first.next_cursor.clone().expect("more remains");

    // The repository changes completely while the client is mid-pull.
    pager.hold("w1", snapshot(5), 1_000);

    // The old cursor must not silently return pages of the new snapshot.
    assert_eq!(
        pager.page("w1", Some(&cursor), 1_000),
        Err(PageRefusal::UnknownCursor),
        "a cursor from a superseded snapshot must be refused, not answered"
    );
}

#[test]
fn an_unknown_cursor_is_refused_rather_than_read_as_the_beginning() {
    // Starting over silently would produce a picture assembled from two snapshots, and the
    // client would have no way to know it happened.
    let pager = StatusPager::new();
    pager.hold("w1", snapshot(10), 1_000);
    assert_eq!(
        pager.page("w1", Some("nonsense"), 1_000),
        Err(PageRefusal::UnknownCursor)
    );
    assert_eq!(
        pager.page("w1", Some("99:0"), 1_000),
        Err(PageRefusal::UnknownCursor)
    );
}

#[test]
fn a_cursor_from_another_workspace_is_refused() {
    let pager = StatusPager::new();
    let a = pager.hold("w1", snapshot(2_000), 1_000);
    pager.hold("w2", snapshot(2_000), 1_000);
    let from_w1 = a.next_cursor.expect("more");
    // w2's generation is its own; w1's cursor must not address it.
    assert_eq!(
        pager.page("w2", Some(&from_w1), 1_000),
        Err(PageRefusal::UnknownCursor)
    );
}

#[test]
fn a_workspace_with_no_snapshot_is_refused() {
    let pager = StatusPager::new();
    assert_eq!(pager.page("never", None, 1_000), Err(PageRefusal::UnknownCursor));
}

#[test]
fn forgetting_a_workspace_releases_its_snapshot() {
    // The snapshot is held only while a pull is in progress. A pager that kept every snapshot
    // for every workspace ever opened would hold the largest status each had ever reported.
    let pager = StatusPager::new();
    pager.hold("w1", snapshot(10), 1_000);
    pager.forget("w1");
    assert_eq!(pager.page("w1", None, 1_000), Err(PageRefusal::UnknownCursor));
}

#[test]
fn an_absurd_limit_is_clamped_rather_than_honoured() {
    // A caller asking for more than the cap gets the cap; §4.1 is the engine's constraint, not
    // a suggestion the client may override.
    let pager = StatusPager::new();
    pager.hold("w1", snapshot(3_000), 1_000);
    let p = pager.page("w1", None, u32::MAX).expect("page");
    assert_eq!(p.changes.len() as u32, MAX_GIT_STATUS_PAGE);
}

#[test]
fn every_page_carries_the_branch() {
    // A client that took the branch only from the first page would be right, but one that took
    // it from the last must also be right — the pages describe one snapshot, branch included.
    let pager = StatusPager::new();
    pager.hold("w1", snapshot(2_500), 1_000);
    let mut cursor: Option<String> = None;
    loop {
        let p = pager.page("w1", cursor.as_deref(), 1_000).expect("page");
        assert_eq!(p.current_branch, BranchPosition::Branch("main".into()));
        match p.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
}
