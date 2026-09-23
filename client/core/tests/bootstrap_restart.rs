//! User Stories 4 and 5: replacement and session continuity, against the real engine.

mod common;

use apex_protocol::wire::{CapabilitySet, HandshakeRequest, SessionId, PROTOCOL_VERSION};
use apex_shell::adapters::outbound::handshake::TransportHandshake;
use apex_shell::application::ports::handshake::HandshakePeer;
use apex_shell::application::ports::transport::{Request, RequestTransport};
use apex_shell::domain::request::RequestOutcome;
use common::connected_engine;
use std::time::Duration;

fn request(resume: Option<&str>) -> HandshakeRequest {
    HandshakeRequest {
        client_version: "0.1.0".into(),
        protocol_version: PROTOCOL_VERSION,
        capabilities: CapabilitySet::of(&["auth/handshake"]),
        resume_session: resume.map(|s| SessionId(s.to_string())),
    }
}

/// T055, T059 — FR-020, FR-024. The engine replaces its own process image and the connection
/// survives: the same transport, the same pipes, the same session.
#[tokio::test]
async fn the_engine_replaces_itself_without_the_connection_dropping() {
    let (t, spawner) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);

    let before = peer
        .handshake(request(None))
        .await
        .expect("handshake")
        .session_id;

    let restarted = t.send(Request::interactive("session/restart", "{}")).await;
    assert!(
        matches!(restarted, RequestOutcome::Answered(_)),
        "the restart must be acknowledged before it happens: {restarted:?}"
    );

    // Same transport, no reconnection. If exec had not preserved the descriptors this would
    // resolve as ConnectionLost.
    let after = peer
        .handshake(request(Some(&before.0)))
        .await
        .expect("a handshake after the restart");

    assert_eq!(
        after.session_id, before,
        "the identity must survive re-execution, or a restart is indistinguishable from a new session"
    );
    assert!(after.resumed, "the client must be told it re-attached");
    assert_eq!(spawner.live_children(), 1, "exec replaces, never forks");
    t.shutdown();
}

/// The finding that trying this early produced.
///
/// `exec` keeps file descriptors and discards memory, so a request already read into the
/// engine's buffer is gone while the connection stays up. Without an explicit refusal the
/// client waits for a reply that can never come — and A-REQ does not cover it, because A-REQ
/// is about a connection that *died*.
#[tokio::test]
async fn a_request_buffered_when_the_engine_restarts_is_refused_rather_than_lost() {
    let (t, _s) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);
    let _ = peer.handshake(request(None)).await.expect("handshake");

    // Issue both without awaiting, so the second is in the engine's buffer when it restarts.
    let (_restart_id, restart) = t.begin(Request::interactive("session/restart", "{}"));
    let mut racing = Request::interactive(
        "auth/handshake",
        r#"{"client_version":"0.1.0","protocol_version":1,"capabilities":[]}"#,
    );
    racing.timeout = Some(Duration::from_secs(5));
    let (_id, raced) = t.begin(racing);

    assert!(matches!(restart.await, RequestOutcome::Answered(_)));

    // Two outcomes are correct, and which one occurs is a legitimate race: the engine either
    // answered before restarting, or refused because it was already restarting. What must
    // never happen is silence — a request left to time out while the connection is healthy.
    match raced.await {
        RequestOutcome::Answered(_) => {}
        RequestOutcome::Failed { code, message } => {
            assert_eq!(code, -32000, "{message}");
            assert!(message.contains("re-issue"), "{message}");
        }
        other => panic!("a buffered request must be answered, not left to time out: {other:?}"),
    }
    t.shutdown();
}

/// T061 — SC-009a, as narrowed. Work survives **re-execution**, not disconnection.
///
/// The original requirement said work continues while no client is connected. Testing it
/// directly showed the engine exits the instant its stdin closes, so that could never have
/// been true — an engine spawned over a channel dies with the channel. F020 carries the
/// detached engine; this asserts the property that does hold, and the one after it pins the
/// limit so nobody assumes the stronger claim again.
#[tokio::test]
async fn work_survives_re_execution_but_the_session_is_bounded_by_its_connection() {
    let (t, spawner) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);
    let before = peer
        .handshake(request(None))
        .await
        .expect("handshake")
        .session_id;

    // Across re-execution: same session, same connection.
    assert!(matches!(
        t.send(Request::interactive("session/restart", "{}")).await,
        RequestOutcome::Answered(_)
    ));
    let after = peer
        .handshake(request(Some(&before.0)))
        .await
        .expect("handshake");
    assert!(after.resumed, "re-execution preserves the session");

    // Across disconnection: the engine goes with it. This is the limit, asserted so it is a
    // decision rather than a surprise.
    t.shutdown();
    for _ in 0..50 {
        if spawner.live_children() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        spawner.live_children(),
        0,
        "the engine's lifetime is its channel's — F020 is what changes this"
    );
}

/// T058 — FR-023. A restart is announced by the engine, never inferred by the client.
#[tokio::test]
async fn a_restart_is_announced_by_the_engine() {
    let (t, _s) = connected_engine(Some("pre-seeded-identity"));
    let peer = TransportHandshake::new(&*t);

    // The engine adopted an identity, so it announced a restart on startup. The client can
    // therefore re-attach to it without having been told the id by any other route.
    let r = peer
        .handshake(request(Some("pre-seeded-identity")))
        .await
        .expect("handshake");
    assert!(r.resumed);
    assert_eq!(r.session_id.0, "pre-seeded-identity");
    t.shutdown();
}

/// T063. A-REQ still holds: an in-flight request dies with its **connection**, even though a
/// session outlives one. The two rules are easy to conflate, and the distinction is the point.
#[tokio::test]
async fn a_request_still_dies_with_its_connection() {
    let (t, _s) = connected_engine(None);
    let mut r = Request::interactive(
        "auth/handshake",
        r#"{"client_version":"0.1.0","protocol_version":1,"capabilities":[]}"#,
    );
    r.timeout = Some(Duration::from_secs(5));
    let (_id, pending) = t.begin(r);
    t.shutdown();
    assert_eq!(
        pending.await,
        RequestOutcome::ConnectionLost,
        "a lost connection resolves its requests rather than leaving them hanging"
    );
}

/// T083 — SC-010.
#[tokio::test]
async fn this_suite_opens_no_network_sockets() {
    let (t, _s) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);
    let _ = peer.handshake(request(None)).await;
    common::assert_no_network();
    t.shutdown();
}

// ---------------------------------------------------------------------------
// User Story 4: replacement and rollback, through doubles.
//
// The real engine cannot express "older, then newer after replacement" without building two
// of them, and the property under test is the client's sequencing, not the engine's version.

use apex_shell::application::ports::handshake::HandshakeError;
use apex_shell::application::use_cases::bootstrap::{Bootstrap, BootstrapOutcome};
use apex_shell::domain::artifact::{Architecture, DeploymentFailure};
use common::{artifact, target, ScriptedDeployer, ScriptedPeer};

fn boot<'a>(
    d: &'a ScriptedDeployer,
    p: &'a ScriptedPeer,
    arts: &'a [apex_shell::domain::artifact::EngineArtifact],
) -> Bootstrap<'a, ScriptedPeer> {
    Bootstrap {
        deployer: d,
        peer: p,
        artifacts: arts,
        target: target(),
        client_version: "0.1.0".into(),
        capabilities: CapabilitySet::of(&["auth/handshake"]),
    }
}

/// T050 — the ordering the deployment contract turns on. `retire_previous` runs **after** a
/// successful handshake, never on promotion: verification says the bytes are right, and only a
/// handshake says the binary runs here.
#[tokio::test]
async fn the_previous_engine_is_retired_only_after_the_replacement_answers() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::answering(&[PROTOCOL_VERSION.saturating_sub(1), PROTOCOL_VERSION]);
    let arts = [artifact(Architecture::LinuxX86_64)];

    let outcome = boot(&d, &p, &arts).establish(None).await;
    assert!(
        matches!(outcome, BootstrapOutcome::Deployed { .. }),
        "{outcome:?}"
    );

    let calls = d.calls();
    let deploy = calls.iter().position(|c| c.starts_with("deploy:"));
    let retire = calls.iter().position(|c| c.starts_with("retire:"));
    assert!(
        deploy.is_some(),
        "the replacement must be deployed: {calls:?}"
    );
    assert!(retire.is_some(), "and the old engine retired: {calls:?}");
    assert!(deploy < retire, "retire must follow deploy: {calls:?}");
}

/// T049 — SC-007, and the case worth writing first. A replacement that **verifies correctly
/// and then will not run** must leave the previous engine in place. A test that only corrupts
/// the artifact never reaches this path, because verification catches that one earlier.
#[tokio::test]
async fn a_replacement_that_verifies_but_will_not_run_leaves_the_old_engine_alone() {
    let d = ScriptedDeployer::default(); // deployment succeeds: the bytes are fine
    let p = ScriptedPeer::failing(HandshakeError::TimedOut); // the binary never answers
    let arts = [artifact(Architecture::LinuxX86_64)];

    let outcome = boot(&d, &p, &arts).replace("0.0.9").await;
    assert!(
        matches!(outcome, BootstrapOutcome::HandshakeFailed(_)),
        "a replacement that will not run is a failed update, got {outcome:?}"
    );
    assert!(
        !d.calls().iter().any(|c| c.starts_with("retire:")),
        "the previous engine must survive a replacement that does not answer: {:?}",
        d.calls()
    );
}

/// T052 — FR-022, SC-011. An engine that starts and dies is retried a bounded number of times.
#[tokio::test]
async fn an_engine_that_never_answers_is_not_redeployed_forever() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::failing(HandshakeError::TimedOut);
    let arts = [artifact(Architecture::LinuxX86_64)];

    match boot(&d, &p, &arts).deploy_then_establish().await {
        BootstrapOutcome::HandshakeFailed(m) => {
            assert!(m.contains("3 attempts"), "the bound must be stated: {m}");
        }
        other => panic!("expected a bounded failure, got {other:?}"),
    }
    assert_eq!(
        p.asked(),
        3,
        "exactly the bound, not one more and not forever"
    );
}

/// T051 — a failed retirement is logged and the session continues. The new engine is running;
/// an orphaned binary costs disk, not correctness.
#[tokio::test]
async fn a_failed_retirement_does_not_fail_the_update() {
    let mut d = ScriptedDeployer::default();
    d.retire_fails = Some(DeploymentFailure::PermissionDenied {
        path: "/home/dev/.apex".into(),
    });
    let p = ScriptedPeer::default();
    let arts = [artifact(Architecture::LinuxX86_64)];

    let outcome = boot(&d, &p, &arts).replace("0.0.9").await;
    assert!(
        matches!(outcome, BootstrapOutcome::Deployed { .. }),
        "the update succeeded even though cleanup did not: {outcome:?}"
    );
}
