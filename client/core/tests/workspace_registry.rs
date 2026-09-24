//! US3: two checkouts of one repository coexist, and a workspace that is gone says so.

mod common;

use apex_shell::adapters::outbound::sqlite::{schema, SqliteWorkspaceCache};
use apex_shell::application::ports::workspace_cache::{Attachment, StoreOutcome, WorkspaceCache};
use apex_shell::application::use_cases::register_workspace::RegisterWorkspace;
use apex_shell::domain::workspace::{EntryKind, FsEntry, Location, RelPath, Sha256, WorkspaceId};
use common::fake_clock::FakeClock;
use std::sync::Arc;

fn store(path: &std::path::Path) -> Arc<SqliteWorkspaceCache> {
    let c = SqliteWorkspaceCache::open(path).expect("open");
    c.migrate_to(schema::CURRENT_VERSION, &mut |_| {})
        .expect("v1");
    Arc::new(c)
}

fn put_file(c: &dyn WorkspaceCache, ws: &WorkspaceId, name: &str, body: &[u8]) -> RelPath {
    c.put_listing(
        ws,
        &RelPath::root(),
        &[FsEntry {
            name: name.into(),
            kind: EntryKind::File,
            size: body.len() as u64,
            modified: 0,
        }],
    )
    .unwrap();
    let p = RelPath::parse(&format!("/{name}")).unwrap();
    let id = c.file_id(ws, &p).unwrap().unwrap();
    assert_eq!(
        c.put_content(&id, body, &Sha256::of(body), 1),
        StoreOutcome::Stored
    );
    p
}

/// US3.1, SC-007. Two workspaces whose display names collide.
#[test]
fn two_workspaces_with_identical_names_each_read_back_their_own_content() {
    let dir = tempfile::tempdir().unwrap();
    let cache = store(&dir.path().join("cache.db"));
    let reg = RegisterWorkspace::new(cache.clone(), Arc::new(FakeClock::at(0)));

    // The ordinary case: the same repository checked out twice, reviewing a branch beside your
    // own work. Both are called "apex".
    let (a, _) = reg
        .open(
            RegisterWorkspace::mint(),
            "apex".into(),
            Location::Remote {
                host: "h".into(),
                base: "/home/dev/apex".into(),
            },
        )
        .unwrap();
    let (b, _) = reg
        .open(
            RegisterWorkspace::mint(),
            "apex".into(),
            Location::Remote {
                host: "h".into(),
                base: "/home/dev/apex-review".into(),
            },
        )
        .unwrap();
    assert_ne!(
        a.id, b.id,
        "identity is minted, never derived from the name"
    );
    assert_eq!(
        a.name, b.name,
        "and names are allowed to collide (A-WORKSPACE)"
    );

    let pa = put_file(cache.as_ref(), &a.id, "main.rs", b"branch A");
    let pb = put_file(cache.as_ref(), &b.id, "main.rs", b"branch B");

    assert_eq!(
        cache.lookup(&a.id, &pa).unwrap().unwrap().bytes,
        b"branch A"
    );
    assert_eq!(
        cache.lookup(&b.id, &pb).unwrap().unwrap().bytes,
        b"branch B",
        "zero cross-reads: a cache keyed on a name or a path would collapse these into one"
    );
}

/// US3.2, FR-011.
#[test]
fn re_opening_a_workspace_attaches_rather_than_building_a_second_projection() {
    let dir = tempfile::tempdir().unwrap();
    let cache = store(&dir.path().join("cache.db"));
    let reg = RegisterWorkspace::new(cache.clone(), Arc::new(FakeClock::at(0)));
    let id = RegisterWorkspace::mint();
    let loc = Location::Remote {
        host: "h".into(),
        base: "/w".into(),
    };

    let (ws, first) = reg.open(id.clone(), "apex".into(), loc.clone()).unwrap();
    assert_eq!(first, Attachment::Created);
    let path = put_file(cache.as_ref(), &ws.id, "main.rs", b"content");

    let (_, second) = reg.open(id.clone(), "apex".into(), loc).unwrap();
    assert_eq!(second, Attachment::Attached);
    assert_eq!(
        cache.lookup(&id, &path).unwrap().unwrap().bytes,
        b"content",
        "attaching must keep what was already cached, or every re-open would refetch the world"
    );
}

/// US3.3, FR-012. The cascade depends on a per-connection pragma, so it is asserted rather than
/// assumed.
#[test]
fn deleting_a_workspace_removes_its_content_and_its_tree() {
    let dir = tempfile::tempdir().unwrap();
    let cache = store(&dir.path().join("cache.db"));
    let reg = RegisterWorkspace::new(cache.clone(), Arc::new(FakeClock::at(0)));
    let (ws, _) = reg
        .open(
            RegisterWorkspace::mint(),
            "apex".into(),
            Location::Remote {
                host: "h".into(),
                base: "/w".into(),
            },
        )
        .unwrap();
    let path = put_file(cache.as_ref(), &ws.id, "main.rs", b"content");

    reg.delete(&ws.id).expect("delete");

    assert!(
        cache
            .list_children(&ws.id, &RelPath::root())
            .unwrap()
            .is_empty(),
        "tree gone"
    );
    assert!(
        cache.lookup(&ws.id, &path).unwrap().is_none(),
        "content gone"
    );
}

/// FR-017. Nothing else in the suite would fail if the store were opened in memory.
#[test]
fn the_projection_survives_a_restart_of_the_application() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cache.db");
    let id = RegisterWorkspace::mint();

    let path = {
        let cache = store(&db);
        let reg = RegisterWorkspace::new(cache.clone(), Arc::new(FakeClock::at(0)));
        let (ws, _) = reg
            .open(
                id.clone(),
                "apex".into(),
                Location::Remote {
                    host: "h".into(),
                    base: "/w".into(),
                },
            )
            .unwrap();
        put_file(cache.as_ref(), &ws.id, "main.rs", b"survives")
    };

    // A new process would open the same file.
    let reopened = SqliteWorkspaceCache::open(&db).expect("reopen");
    assert_eq!(
        reopened
            .lookup(&id, &path)
            .unwrap()
            .expect("content survives")
            .bytes,
        b"survives"
    );
    assert_eq!(
        reopened.list_children(&id, &RelPath::root()).unwrap().len(),
        1
    );
}

/// A workspace deleted then re-registered under a fresh identity does not inherit the old one's
/// content — which is what keeps identity meaningful.
#[test]
fn a_new_identity_starts_with_an_empty_projection() {
    let dir = tempfile::tempdir().unwrap();
    let cache = store(&dir.path().join("cache.db"));
    let reg = RegisterWorkspace::new(cache.clone(), Arc::new(FakeClock::at(0)));
    let loc = Location::Remote {
        host: "h".into(),
        base: "/w".into(),
    };

    let (first, _) = reg
        .open(RegisterWorkspace::mint(), "apex".into(), loc.clone())
        .unwrap();
    put_file(cache.as_ref(), &first.id, "main.rs", b"old");
    reg.delete(&first.id).unwrap();

    let (second, _) = reg
        .open(RegisterWorkspace::mint(), "apex".into(), loc)
        .unwrap();
    assert!(
        cache
            .list_children(&second.id, &RelPath::root())
            .unwrap()
            .is_empty(),
        "same path, same name, new identity — and no inherited content"
    );
}
