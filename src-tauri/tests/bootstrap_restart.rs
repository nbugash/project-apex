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
