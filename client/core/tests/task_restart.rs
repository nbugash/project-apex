//! SC-023's client half: every identity in the reloaded store attaches.
//!
//! The store is the only thing that crosses a client restart, so what is asserted is that the
//! restart reattaches to **exactly** what was written and nothing else -- not to a list held in
//! memory by the harness, which would be a test of the harness.

mod common;

use apex_protocol::wire::TaskId;
use apex_shell::adapters::outbound::json_session_store::JsonFileSessionStore;
use apex_shell::application::ports::session_store::SessionStore;
use apex_shell::application::use_cases::observe_connection::{reattach, Outcome};
use apex_shell::domain::session::{PersistedSession, PersistedTask};
use apex_shell::domain::workspace::WorkspaceId;
use common::reconnect::{running, Recorder};

#[tokio::test]
async fn every_identity_in_the_reloaded_store_attaches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.json");

    // The client before the restart: it wrote what it was running.
    {
        let store = JsonFileSessionStore::new(path.clone());
        let session = PersistedSession {
            tasks: vec![
                PersistedTask {
                    task_id: "build-01".into(),
                    workspace_id: "ws1".into(),
                },
                PersistedTask {
                    task_id: "test-02".into(),
                    workspace_id: "ws1".into(),
                },
            ],
            ..PersistedSession::default()
        };
        store.save(&session).expect("save");
    }

    // The client after: nothing in memory, only what is on disk.
    let reloaded = JsonFileSessionStore::new(path)
        .load()
        .expect("the store must reload");
    let identities: Vec<TaskId> = reloaded
        .tasks
        .iter()
        .map(|t| TaskId(t.task_id.clone()))
        .collect();

    let r = Recorder::default();
    r.answer("build-01", running(1024));
    r.answer("test-02", running(0));

    let report = reattach(
        &WorkspaceId("ws1".into()),
        &[],
        &identities,
        (120, 40),
        &r,
        &r,
    )
    .await;

    let attached: Vec<String> = r
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("attach:"))
        .collect();
    assert_eq!(
        attached,
        vec!["attach:build-01", "attach:test-02"],
        "the restart did not reattach to exactly what the store held"
    );
    assert_eq!(report.outcomes.len(), 2);
    assert_eq!(report.retained(), 1024);
    // Remembered, so no enumeration: the store is what it knows.
    assert!(
        !r.calls().iter().any(|c| c == "list"),
        "a client with a store listed anyway"
    );
}

#[tokio::test]
async fn a_client_whose_store_is_gone_finds_its_tasks_by_listing() {
    // The other half of SC-023, and it **discards** the store rather than ignoring it. A harness
    // that kept the identities in a variable would be testing nothing: the claim is that a client
    // with no record can still find them.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.json");
    {
        let store = JsonFileSessionStore::new(path.clone());
        store
            .save(&PersistedSession {
                tasks: vec![PersistedTask {
                    task_id: "orphan".into(),
                    workspace_id: "ws1".into(),
                }],
                ..PersistedSession::default()
            })
            .expect("save");
    }
    std::fs::remove_file(&path).expect("remove");

    let reloaded = JsonFileSessionStore::new(path).load().unwrap_or_default();
    assert!(
        reloaded.tasks.is_empty(),
        "the store was supposed to be gone"
    );

    let r = Recorder::default();
    r.list_returns("orphan");
    r.answer("orphan", running(64));

    let report = reattach(&WorkspaceId("ws1".into()), &[], &[], (80, 24), &r, &r).await;

    assert!(
        r.calls().iter().any(|c| c == "list"),
        "a client with no store must list to find what it left running"
    );
    assert_eq!(
        report.outcomes,
        vec![Outcome::Survived {
            task: TaskId("orphan".into()),
            retained: 64,
        }]
    );
}
