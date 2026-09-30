//! `git/recentlyChanged`'s wire spelling, asserted against hand-written JSON rather than trusted to
//! the derive (A-WIRECASE): a rename on a field is invisible until the other side cannot read it.

use apex_protocol::wire::{RecentlyChangedParams, RecentlyChangedResult, WorkspaceId};

#[test]
fn params_are_snake_case_and_commits_is_optional() {
    let with: RecentlyChangedParams =
        serde_json::from_str(r#"{"workspace_id":"w1","commits":7}"#).expect("parse");
    assert_eq!(
        with,
        RecentlyChangedParams {
            workspace_id: WorkspaceId("w1".into()),
            commits: Some(7)
        }
    );
    let without: RecentlyChangedParams =
        serde_json::from_str(r#"{"workspace_id":"w1"}"#).expect("parse");
    assert_eq!(without.commits, None);
    assert_eq!(
        serde_json::to_string(&without).expect("encode"),
        r#"{"workspace_id":"w1"}"#,
        "an absent count is absent on the wire, not null"
    );
}

#[test]
fn the_result_is_a_list_of_paths_under_one_key() {
    let r = RecentlyChangedResult {
        paths: vec!["/a.rs".into(), "/src/b.rs".into()],
    };
    let text = serde_json::to_string(&r).expect("encode");
    assert_eq!(text, r#"{"paths":["/a.rs","/src/b.rs"]}"#);
    assert_eq!(
        serde_json::from_str::<RecentlyChangedResult>(&text).expect("parse"),
        r
    );
}
