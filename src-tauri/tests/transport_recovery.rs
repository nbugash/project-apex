//! User Story 1 — reach the remote machine, and keep reaching it.
//!
//! Every test runs against the mock daemon: no network, no remote host, no engine (SC-010).

mod common;

use apex_shell::adapters::outbound::openssh::SshTransport;
use apex_shell::application::ports::spawner::SpawnError;
use apex_shell::application::ports::transport::{Request, RequestTransport};
use apex_shell::domain::connection::ConnectionState;
use apex_shell::domain::request::RequestOutcome;
use common::{connected, spec, MockSpawner, ScriptedSpawner};
use std::sync::Arc;
use std::time::Duration;

/// T021 — SC-002. One authentication for the session, however many channels open.
#[tokio::test]
async fn one_connection_serves_every_channel() {
    let (t, spawner) = connected("echo");

    // Two logical channels: two callers sharing the transport.
    let a = t.send(Request::interactive("a/one", "{}")).await;
    let b = t.send(Request::background("b/two", "{}")).await;

    assert!(a.is_answered(), "first channel: {a:?}");
    assert!(b.is_answered(), "second channel: {b:?}");
    assert_eq!(
        spawner.live_children(),
        1,
        "a second channel must attach to the existing connection, not authenticate again"
    );
    t.shutdown();
}

/// T022 — SC-003. Nothing outlives the application.
#[tokio::test]
async fn teardown_leaves_no_child_behind() {
    let (t, spawner) = connected("echo");
    assert!(t
        .send(Request::interactive("ping", "{}"))
        .await
        .is_answered());
    assert_eq!(spawner.live_children(), 1);

    t.shutdown();

    // The child may take a moment to reap after its pipes close.
    for _ in 0..50 {
        if spawner.live_children() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("a child process outlived the transport");
}

/// T023 — SC-012. Requests outstanding when the link dies must resolve, not hang.
#[tokio::test]
async fn losing_the_connection_resolves_everything_outstanding() {
    // The mock drops every reply, so requests are genuinely in flight when the pipe closes.
    let spawner = Arc::new(MockSpawner::new("drop=1"));
    let t = Arc::new(SshTransport::new(spawner.clone(), spec()));
    t.connect().expect("connect");

    let waiting = {
        let t = t.clone();
        tokio::spawn(async move {
            t.send(Request {
                timeout: Some(Duration::from_secs(30)),
                ..Request::interactive("never/answered", "{}")
            })
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(t.outstanding(), 1, "the request should be in flight");

    t.shutdown();

    let outcome = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the request hung instead of resolving")
        .expect("task panicked");
    assert_eq!(
        outcome,
        RequestOutcome::ConnectionLost,
        "an in-flight request must die with the connection, not wait for a reply that cannot come"
    );
    assert_eq!(t.outstanding(), 0, "nothing may survive a lost connection");
}

/// T024. A tight retry loop passes a "did it reconnect" assertion and is still wrong.
#[test]
fn the_retry_interval_grows_between_attempts() {
    use apex_shell::application::use_cases::supervise::{Backoff, MAX_DELAY};

    let mut b = Backoff::default();
    let mut ceilings = Vec::new();
    for _ in 0..8 {
        ceilings.push(b.next_ceiling());
        b.next_delay(1.0);
    }

    assert!(
        ceilings.windows(2).all(|w| w[1] >= w[0]),
        "the interval must never shrink: {ceilings:?}"
    );
    assert!(
        ceilings[4] > ceilings[0],
        "the interval must actually grow, not merely repeat: {ceilings:?}"
    );
    assert_eq!(
        *ceilings.last().unwrap(),
        MAX_DELAY,
        "and must stop growing, so a laptop waking after an hour reconnects in seconds"
    );
}

/// T025 — SC-012. Recovery needs no caller action.
#[tokio::test]
async fn the_connection_re_establishes_without_caller_action() {
    let spawner = Arc::new(MockSpawner::new("echo"));
    let t = Arc::new(SshTransport::new(spawner.clone(), spec()));

    t.connect().expect("first connect");
    assert!(t
        .send(Request::interactive("before", "{}"))
        .await
        .is_answered());

    // Lose it.
    t.shutdown();
    assert_eq!(t.state(), ConnectionState::Disconnected);

    // The supervisor's job, done here explicitly: reconnect, with no caller involvement.
    t.connect().expect("reconnect");
    assert_eq!(t.state(), ConnectionState::Connected);
    assert!(
        t.send(Request::interactive("after", "{}"))
            .await
            .is_answered(),
        "the transport must be usable again after recovery"
    );
    t.shutdown();
}

/// T027 — FR-004, SC-009, US1 acceptance scenario 4.
///
/// The mock stalls and then closes, which is what `ssh` does when its keepalive gives up.
/// That EOF is the single observation the transport has; the flags that bound the time to it
/// are asserted separately, in the spawner's own unit test, because the mock has no socket.
#[tokio::test]
async fn a_silent_link_is_reported_lost_rather_than_hanging() {
    let spawner = Arc::new(MockSpawner::new("stall=200"));
    let t = Arc::new(SshTransport::new(spawner.clone(), spec()));
    t.connect().expect("connect");

    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        t.send(Request {
            timeout: Some(Duration::from_secs(30)),
            ..Request::interactive("into/the/void", "{}")
        }),
    )
    .await
    .expect("the transport hung on a silent link instead of reporting it lost");

    assert_eq!(
        outcome,
        RequestOutcome::ConnectionLost,
        "a link that goes silent and closes must surface as lost"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "loss must be reported promptly once the child ends, not after the request's own timeout"
    );
    t.shutdown();
}

/// T028 — FR-005, US1 acceptance scenario 5.
#[test]
fn startup_refuses_an_absent_or_too_old_ssh() {
    let absent = SshTransport::new(
        Arc::new(ScriptedSpawner::with_version(Err(SpawnError::NotFound))),
        spec(),
    );
    match absent.preflight() {
        Err(SpawnError::NotFound) => {}
        other => panic!("an absent ssh must be refused at startup, got {other:?}"),
    }

    let old = SshTransport::new(
        Arc::new(ScriptedSpawner::with_version(Err(SpawnError::TooOld {
            found: "OpenSSH_6.6p1".into(),
            required: "OpenSSH_6.7".into(),
        }))),
        spec(),
    );
    match old.preflight() {
        Err(SpawnError::TooOld { found, required }) => {
            // The message names what was found: a user told only "too old" cannot tell
            // whether they upgraded the thing that mattered.
            assert!(
                found.contains("6.6"),
                "the message must name what was found"
            );
            assert!(required.contains("6.7"));
        }
        other => panic!("an old ssh must be refused at startup, got {other:?}"),
    }
}

/// T029 — contracts/transport.md: a subscriber's view converges on the transport's.
///
/// Deliberately not "every intermediate state is delivered": a `watch` channel coalesces,
/// and that is right for the consumer — a status bar flashing "Connecting" for five
/// milliseconds is noise. What must hold is that a subscriber is woken by every change and
/// never left reading a stale value.
#[tokio::test]
async fn a_subscriber_never_reads_a_stale_state() {
    let spawner = Arc::new(MockSpawner::new("echo"));
    let t = Arc::new(SshTransport::new(spawner, spec()));

    let mut rx = t.observe();
    assert_eq!(*rx.borrow_and_update(), ConnectionState::Unknown);

    t.connect().expect("connect");
    rx.changed()
        .await
        .expect("a change must wake the subscriber");
    assert_eq!(
        *rx.borrow_and_update(),
        t.state(),
        "the subscriber must read what the transport currently is"
    );
    assert_eq!(t.state(), ConnectionState::Connected);

    // Retrying carries its progress, so a caller can show that something is happening.
    t.report_retrying(3, 8);
    rx.changed()
        .await
        .expect("a change must wake the subscriber");
    assert_eq!(
        *rx.borrow_and_update(),
        ConnectionState::Retrying {
            attempt: 3,
            next_in_secs: 8
        }
    );

    t.shutdown();
    rx.changed()
        .await
        .expect("a change must wake the subscriber");
    assert_eq!(*rx.borrow_and_update(), ConnectionState::Disconnected);
    assert_eq!(*rx.borrow(), t.state(), "the views must agree at the end");
}
