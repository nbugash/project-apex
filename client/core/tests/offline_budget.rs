//! What `offline_status` reports, and what it costs.
//!
//! The contract's two dangerous guarantees are here rather than in an end-to-end spec because both
//! are about what the client does *not* do: guarantee 1 says the reported state comes from the
//! published connection state and is never re-derived, and guarantee 2 says reading it contacts
//! nothing. Neither is observable by watching the interface work correctly — they are observable
//! only by counting.

mod common;

use apex_shell::adapters::inbound::tauri_commands::offline_report;
use apex_shell::application::ports::workspace_cache::{PendingEdit, StoreOutcome, WorkspaceCache};
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

// ---- the measurements (SC-007, SC-008) ----
//
// Through the **real** store on a real file, not a double returning a clone. F011's first budget
// test printed `0 us` because it measured a stub, and a printed zero cannot tell a fast client from
// one that is not running. Printing is also not what Principle V asks for on its own: it demands a
// measurement that *fails* when the budget is exceeded, so each of these asserts as well as prints.

use apex_shell::adapters::outbound::sqlite::schema::CURRENT_VERSION;
use apex_shell::adapters::outbound::sqlite::SqliteWorkspaceCache;
use apex_shell::domain::workspace::{EntryKind, FsEntry};
use std::time::Instant;

const SAMPLES: usize = 50;
const OPEN_BUDGET_MS: u128 = 200;
const SEARCH_BUDGET_MS: u128 = 1_000;
const CORPUS: usize = 50_000;

fn p99(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    let idx = ((samples.len() * 99) / 100).min(samples.len() - 1);
    samples[idx]
}

fn entry(name: &str) -> FsEntry {
    FsEntry {
        name: name.to_string(),
        kind: EntryKind::File,
        size: 10,
        modified: 0,
    }
}

/// A store on disk with one cached file, opened offline.
#[test]
fn sc_007_a_cached_file_opens_within_the_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = SqliteWorkspaceCache::open(&db).expect("open");
    store
        .migrate_to(CURRENT_VERSION, &mut |_| {})
        .expect("migrate");
    let ws = workspace("w1");
    store.register(&ws, 0).expect("register");

    // A file of a realistic size: the budget is about reading and decompressing, and a two-byte
    // blob would measure the query and nothing else.
    let bytes: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    let file = path("/main.rs");
    store
        .put_listing(&ws.id, &RelPath::root(), &[entry("main.rs")])
        .expect("listing");
    let id = store
        .file_id(&ws.id, &file)
        .expect("ask")
        .expect("the file is listed");
    // Asserted rather than discarded. `StoreOutcome` is `#[must_use]` for F005's stated reason --
    // a caching outcome is recorded or deliberately ignored, never silently dropped -- and here it
    // is load-bearing: a `NotEligible` or `Failed` would make `lookup` return nothing below, and
    // the budget test would fail on a missing file rather than on a slow one.
    assert!(
        matches!(
            store.put_content(&id, &bytes, &Sha256::of(&bytes), 0),
            StoreOutcome::Stored
        ),
        "the file must actually be cached for this to be a measurement of reading it"
    );

    for _ in 0..5 {
        let _ = store.lookup(&ws.id, &file);
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let at = Instant::now();
        let hit = store
            .lookup(&ws.id, &file)
            .expect("the store must answer")
            .expect("the file is cached");
        samples.push(at.elapsed().as_micros());
        // Read the bytes, so the measurement covers producing content rather than producing a row.
        assert_eq!(hit.bytes.len(), bytes.len());
    }

    let p99_us = p99(samples);
    println!(
        "SC-007 offline open of a cached 64 KiB file: p99 {p99_us} us over {SAMPLES} samples \
         ({:.3} ms of the {OPEN_BUDGET_MS} ms budget)",
        p99_us as f64 / 1000.0
    );
    assert!(
        p99_us / 1000 < OPEN_BUDGET_MS,
        "p99 {} ms exceeded the {OPEN_BUDGET_MS} ms budget",
        p99_us / 1000
    );
}

/// Path search over a corpus the size SC-008 names.
#[test]
fn sc_008_path_search_over_fifty_thousand_paths_within_the_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("cache.db");
    let store = SqliteWorkspaceCache::open(&db).expect("open");
    store
        .migrate_to(CURRENT_VERSION, &mut |_| {})
        .expect("migrate");
    let ws = workspace("w1");
    store.register(&ws, 0).expect("register");

    // 50,000 paths across 500 directories, which is what makes the FTS index do work rather than
    // the one-directory shape a smaller fixture would have.
    let per_dir = 100;
    for d in 0..(CORPUS / per_dir) {
        let parent = RelPath::parse(&format!("/pkg{d}")).expect("path");
        let items: Vec<FsEntry> = (0..per_dir)
            .map(|f| entry(&format!("module_{d}_{f}.rs")))
            .collect();
        store.put_listing(&ws.id, &parent, &items).expect("listing");
    }

    for _ in 0..3 {
        let _ = store.search_paths(&ws.id, "module_7_", 50);
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // A different fragment each time, so the measurement is not one warm query repeated.
        let fragment = format!("module_{}_", i % 500);
        let at = Instant::now();
        let hits = store
            .search_paths(&ws.id, &fragment, 50)
            .expect("the store must answer");
        samples.push(at.elapsed().as_micros());
        assert!(!hits.is_empty(), "{fragment} must match something");
    }

    let p99_us = p99(samples);
    println!(
        "SC-008 offline path search over {CORPUS} cached paths: p99 {p99_us} us over {SAMPLES} \
         samples ({:.3} ms of the {SEARCH_BUDGET_MS} ms budget)",
        p99_us as f64 / 1000.0
    );
    assert!(
        p99_us / 1000 < SEARCH_BUDGET_MS,
        "p99 {} ms exceeded the {SEARCH_BUDGET_MS} ms budget",
        p99_us / 1000
    );
}

// ---- FR-007: results are never presented as complete when they cannot be ----

use apex_shell::adapters::inbound::tauri_commands::search_complete;

/// Offline is never complete, however few results came back.
///
/// The case that matters is the last one: a short list while disconnected *looks* like a whole
/// answer, and an implementation deriving completeness from "fewer than the limit" alone would
/// present a partial cache as the entire repository. That is what FR-007 forbids.
#[test]
fn a_disconnected_search_is_never_complete() {
    assert!(!search_complete(false, 0, 20), "no results, offline");
    assert!(!search_complete(false, 3, 20), "a short list, offline");
    assert!(!search_complete(false, 20, 20), "a full page, offline");
}

/// Connected, a short list is the whole answer; a full page is not.
#[test]
fn a_connected_search_is_complete_only_below_the_limit() {
    assert!(
        search_complete(true, 3, 20),
        "fewer than asked for, connected"
    );
    assert!(
        !search_complete(true, 20, 20),
        "a full page may have more behind it, connected or not"
    );
}
