//! `git/*` on the wire.
//!
//! Asserted against **hand-written JSON** rather than against a round trip of the structs. A
//! round trip is symmetric: rename a field and serialisation and deserialisation rename
//! together, agreeing with each other and with nothing else. The engine and the client share
//! these types, so that symmetry hides exactly the change that would break a third party
//! implementing §4.8 — and A-WIRECASE records what happened when the spelling convention was
//! reasoned about rather than read.

use apex_protocol::wire::{
    BranchPosition, GitChange, GitDiffResult, GitStatusKind, GitStatusParams, GitStatusResult,
    GitStatusUpdate, MAX_GIT_STATUS_PAGE,
};

#[test]
fn status_params_are_snake_case_on_the_wire() {
    // §4.8's tables are camelCase for readability; the wire carries snake_case, and §4.8 says so
    // three paragraphs below them. A frame written by hand is the only thing that checks it.
    let frame = r#"{"workspace_id":"ws1","cursor":"c-7","limit":250}"#;
    let p: GitStatusParams = serde_json::from_str(frame).expect("the engine must accept this");
    assert_eq!(p.workspace_id.0, "ws1");
    assert_eq!(p.cursor.as_deref(), Some("c-7"));
    assert_eq!(p.limit, Some(250));
}

#[test]
fn camel_case_params_are_refused_rather_than_silently_defaulted() {
    // If camelCase ever parses, the convention has moved and §4.8 has to move with it --
    // deliberately, not by accident.
    let frame = r#"{"workspaceId":"ws1"}"#;
    assert!(
        serde_json::from_str::<GitStatusParams>(frame).is_err(),
        "camelCase parsed, so the wire spelling changed without §4.8 changing"
    );
}

#[test]
fn a_first_page_asks_for_nothing_and_says_so() {
    // Absent cursor and absent limit are how a client asks for the beginning. Emitting nulls
    // instead would make "no cursor" and "a cursor that is null" two spellings of one thing.
    let json = serde_json::to_string(&GitStatusParams {
        workspace_id: apex_protocol::wire::WorkspaceId("ws1".into()),
        cursor: None,
        limit: None,
    })
    .expect("encode");
    assert_eq!(json, r#"{"workspace_id":"ws1"}"#);
}

#[test]
fn the_five_states_are_spelled_as_4_8_spells_them() {
    // Upper case on the wire, because §4.8 writes them that way. A test that compared the enum
    // against itself would accept any spelling at all.
    let frame = r#"{"path":"/src/a.rs","status":"MODIFIED"}"#;
    let c: GitChange = serde_json::from_str(frame).expect("decode");
    assert_eq!(c.status, GitStatusKind::Modified);
    for (kind, text) in [
        (GitStatusKind::Modified, "MODIFIED"),
        (GitStatusKind::Untracked, "UNTRACKED"),
        (GitStatusKind::Staged, "STAGED"),
        (GitStatusKind::Deleted, "DELETED"),
        (GitStatusKind::Conflict, "CONFLICT"),
    ] {
        let encoded = serde_json::to_string(&kind).expect("encode");
        assert_eq!(encoded, format!("\"{text}\""));
    }
}

#[test]
fn a_sixth_state_is_refused() {
    // The set is closed. A sixth value arriving from a newer engine must be an error the client
    // can see, not a variant it silently maps onto one of the five.
    let frame = r#"{"path":"/a","status":"IGNORED"}"#;
    assert!(serde_json::from_str::<GitChange>(frame).is_err());
}

#[test]
fn the_absence_of_a_branch_is_its_own_case_and_not_an_empty_name() {
    // The distinction research.md found: git reports a detached head as the literal
    // `(detached)` where a name goes. Three cases on the wire means no consumer can mistake
    // that string for a branch somebody named.
    let on_branch: BranchPosition =
        serde_json::from_str(r#"{"kind":"branch","value":"main"}"#).expect("branch");
    let detached: BranchPosition =
        serde_json::from_str(r#"{"kind":"detached","value":"9581378"}"#).expect("detached");
    let nothing: BranchPosition = serde_json::from_str(r#"{"kind":"none"}"#).expect("none");

    assert_eq!(on_branch, BranchPosition::Branch("main".into()));
    assert_eq!(detached, BranchPosition::Detached("9581378".into()));
    assert_eq!(nothing, BranchPosition::None);
    assert_ne!(detached, BranchPosition::Branch("9581378".into()));
}

#[test]
fn a_final_page_carries_no_cursor_at_all() {
    // "Present exactly when more remain" is the guarantee the whole accumulation rests on. A
    // null cursor on the last page would make an absent cursor and an empty one two spellings,
    // and a client checking for presence would never commit (A-GITPAGE).
    let json = serde_json::to_string(&GitStatusResult {
        current_branch: BranchPosition::Branch("main".into()),
        changes: vec![],
        next_cursor: None,
    })
    .expect("encode");
    assert!(!json.contains("next_cursor"), "{json}");
}

#[test]
fn a_non_final_page_carries_one() {
    let frame = r#"{"current_branch":{"kind":"branch","value":"main"},"changes":[],"next_cursor":"c-2"}"#;
    let r: GitStatusResult = serde_json::from_str(frame).expect("decode");
    assert_eq!(r.next_cursor.as_deref(), Some("c-2"));
}

#[test]
fn the_notification_carries_a_workspace_the_result_does_not() {
    // A notification arrives unsolicited, so it must say which workspace it is about; a result
    // answers a request that already named one. Getting this backwards produces an update the
    // client cannot attribute.
    let frame = r#"{"workspace_id":"ws1","current_branch":{"kind":"none"},"changes":[]}"#;
    let u: GitStatusUpdate = serde_json::from_str(frame).expect("decode");
    assert_eq!(u.workspace_id.0, "ws1");
    assert_eq!(u.current_branch, BranchPosition::None);
}

#[test]
fn a_diff_has_nowhere_to_put_file_content() {
    // FR-021 asserted on the *shape*: the encoded form of a fully populated diff contains only
    // coordinates. A field added later to carry text would break this, which is the point.
    let d = GitDiffResult {
        added: vec![[5, 7]],
        deleted: vec![4],
        modified: vec![[2, 2]],
    };
    let json = serde_json::to_string(&d).expect("encode");
    assert_eq!(json, r#"{"added":[[5,7]],"deleted":[4],"modified":[[2,2]]}"#);
}

#[test]
fn a_deletion_is_a_position_and_an_addition_is_a_range() {
    // Removed lines are not in the new file, so there is no end to give. Modelling both as
    // ranges would force an invented end for every deletion, and readers would then have to
    // know which end was real.
    let frame = r#"{"added":[[1,3]],"deleted":[9],"modified":[]}"#;
    let d: GitDiffResult = serde_json::from_str(frame).expect("decode");
    assert_eq!(d.added, vec![[1, 3]]);
    assert_eq!(d.deleted, vec![9]);
}

#[test]
fn the_page_size_is_read_directorys_and_not_a_second_number() {
    // Written as a literal on purpose: the constant and plan.md's *Fixed Quantities* must agree,
    // and a test that read the constant on both sides would agree with itself.
    assert_eq!(MAX_GIT_STATUS_PAGE, 1000);
    assert_eq!(
        MAX_GIT_STATUS_PAGE as u64,
        apex_protocol::wire::MAX_DIRECTORY_PAGE as u64,
        "status pages and directory pages are the same problem; two numbers would be two things \
         to keep in step"
    );
}
