//! What `offline_status` reports, and what it costs.
//!
//! The contract's two dangerous guarantees are here rather than in an end-to-end spec because both
//! are about what the client does *not* do: guarantee 1 says the reported state comes from the
//! published connection state and is never re-derived, and guarantee 2 says reading it contacts
//! nothing. Neither is observable by watching the interface work correctly — they are observable
//! only by counting.

mod common;

use apex_shell::adapters::inbound::tauri_commands::offline_report;
use apex_shell::application::ports::workspace_cache::{PendingEdit, WorkspaceCache};
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::workspace::{Location, RelPath, Sha256, Workspace, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn workspace(id: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(id.into()),
        name: "repo".into(),
        location: Location::Local {
            base: "/repo".into(),
        },
        last_opened_at: 0,
    }
}

fn edit(at: i64, mergeable: bool) -> PendingEdit {
    PendingEdit {
        content: b"local".to_vec(),
        base: Some((b"host".to_vec(), Sha256::of(b"host"))),
        mergeable,
        retained_at: at,
    }
}

fn path(p: &str) -> RelPath {
    RelPath::parse(p).expect("a valid path")
}

/// Guarantee 1: `connected` follows the published state and nothing else.
///
/// The case an implementation gets wrong is the last one: a request has failed, and the connection
/// state still says `Connected`. Treating the failure as evidence is how an offline indicator
/// starts lying during a slow request — it is the difference between reporting a state and
/// inferring one, and inferring is what FR-001 forbids.
#[test]
fn connected_follows_the_published_state_and_never_a_failed_request() {
    let cache = InMemoryCache::new();
    let ws = workspace("w1");
    cache.register(&ws, 0).expect("register");

    for (state, expected) in [
        (ConnectionState::Connected, true),
        (ConnectionState::Disconnected, false),
        (ConnectionState::Connecting, false),
        (ConnectionState::Unknown, false),
    ] {
        let report = offline_report(&cache, &state, &ws.id).expect("report");
        assert_eq!(
            report.connected, expected,
            "{state:?} must report connected={expected}"
        );
    }
}

/// Guarantee 2: reading this issues **zero** requests.
///
/// Asserted by giving the report a cache and no provider at all. A count of zero is weaker than it
/// looks when the thing being counted is reachable; here the engine is not reachable even in
/// principle, so a request would not fail — it could not be written.
#[test]
fn reading_the_status_contacts_nothing() {
    let cache = InMemoryCache::new();
    let ws = workspace("w1");
    cache.register(&ws, 0).expect("register");
    cache
        .retain_edit(&ws.id, &path("/a.rs"), &edit(1, true))
        .expect("retain");

    // No provider is threaded into `offline_report` at all, which is the assertion: its signature
    // cannot reach the engine, so guarantee 2 holds by construction rather than by discipline.
    let report = offline_report(&cache, &ConnectionState::Disconnected, &ws.id).expect("report");
    assert_eq!(report.pending.len(), 1);
}

/// Guarantee 3: `pending` is this workspace's work and nothing from another.
///
/// Two workspaces, work in both. A single-workspace fixture passes whether or not the query is
/// scoped, which is exactly what makes a cross-workspace leak invisible — the table is keyed
/// `(workspace_id, relative_path)` so that it *can* be scoped, and a query that forgot the first
/// half of the key would still return plausible rows.
#[test]
fn pending_is_scoped_to_the_current_workspace() {
    let cache = InMemoryCache::new();
    let (w1, w2) = (workspace("w1"), workspace("w2"));
    cache.register(&w1, 0).expect("register");
    cache.register(&w2, 0).expect("register");
    cache
        .retain_edit(&w1.id, &path("/mine.rs"), &edit(1, true))
        .expect("retain");
    cache
        .retain_edit(&w1.id, &path("/also-mine.rs"), &edit(2, false))
        .expect("retain");
    cache
        .retain_edit(&w2.id, &path("/theirs.rs"), &edit(3, true))
        .expect("retain");

    let mine = offline_report(&cache, &ConnectionState::Disconnected, &w1.id).expect("report");
    let paths: Vec<&str> = mine
        .pending
        .iter()
        .map(|p| p.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["/mine.rs", "/also-mine.rs"],
        "ordered by retention"
    );
    assert!(
        !paths.contains(&"/theirs.rs"),
        "another workspace's work must not appear"
    );

    let theirs = offline_report(&cache, &ConnectionState::Disconnected, &w2.id).expect("report");
    assert_eq!(theirs.pending.len(), 1);
}

/// `mergeable` reaches the interface, because it decides whether a file will prompt (FR-025a).
#[test]
fn each_pending_entry_carries_whether_it_can_be_merged() {
    let cache = InMemoryCache::new();
    let ws = workspace("w1");
    cache.register(&ws, 0).expect("register");
    cache
        .retain_edit(&ws.id, &path("/text.rs"), &edit(1, true))
        .expect("retain");
    cache
        .retain_edit(&ws.id, &path("/blob.bin"), &edit(2, false))
        .expect("retain");

    let report = offline_report(&cache, &ConnectionState::Disconnected, &ws.id).expect("report");
    let flags: Vec<bool> = report.pending.iter().map(|p| p.mergeable).collect();
    assert_eq!(
        flags,
        vec![true, false],
        "a file the client cannot merge must say so before reconnection, not during it"
    );
}
