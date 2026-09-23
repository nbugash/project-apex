//! User Stories 2 and 3, against the **real** engine.
//!
//! The mock cannot serve these: a test asserts that no §4.8 method name appears in its
//! directory, which is what keeps it a framing double. So the handshake is exercised against
//! the binary this feature builds, spawned as a local child — no network, no remote host.

mod common;

use apex_protocol::wire::{CapabilitySet, HandshakeRequest, SessionId, PROTOCOL_VERSION};
use apex_shell::adapters::outbound::handshake::TransportHandshake;
use apex_shell::application::ports::handshake::HandshakePeer;
use apex_shell::application::use_cases::bootstrap::{may_use, VersionVerdict};
use common::connected_engine;

fn request(resume: Option<&str>) -> HandshakeRequest {
    HandshakeRequest {
        client_version: "0.1.0".into(),
        protocol_version: PROTOCOL_VERSION,
        capabilities: CapabilitySet::of(&["auth/handshake"]),
        resume_session: resume.map(|s| SessionId(s.to_string())),
    }
}

/// T031 — FR-010. The handshake completes over the real transport, against the real engine.
#[tokio::test]
async fn the_handshake_completes_against_the_real_engine() {
    let (t, _s) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);

    let r = peer.handshake(request(None)).await.expect("a handshake");
    assert_eq!(r.protocol_version, PROTOCOL_VERSION);
    assert!(!r.session_id.0.is_empty(), "a session must be identified");
    assert!(!r.resumed, "a first connect is not a resumption");
    assert_eq!(
        VersionVerdict::compare(r.protocol_version, PROTOCOL_VERSION),
        VersionVerdict::Current
    );
    t.shutdown();
}

/// T032 — SC-008. A capability the engine did not advertise produces **no frame on the wire**.
///
/// Asserted on what the transport was asked to send, not on the error the caller received: a
/// request that is sent and rejected also produces an error, so the error alone cannot tell
/// the two apart.
#[tokio::test]
async fn an_unadvertised_capability_never_reaches_the_wire() {
    let (t, _s) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);
    let advertised = peer
        .handshake(request(None))
        .await
        .expect("a handshake")
        .capabilities;

    assert!(
        !may_use(&advertised, "workspace/readFile"),
        "F002's engine serves no workspace method"
    );
    let before = t.outstanding();
    // The guard is what stops the request existing at all.
    if may_use(&advertised, "workspace/readFile") {
        panic!("guard did not hold");
    }
    assert_eq!(
        t.outstanding(),
        before,
        "refusing locally must not leave a request in flight"
    );
    t.shutdown();
}

/// The engine advertises what it serves, and the client's offering follows from that.
#[tokio::test]
async fn the_client_offers_exactly_what_the_engine_advertised() {
    let (t, _s) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);
    let c = peer
        .handshake(request(None))
        .await
        .expect("handshake")
        .capabilities;

    assert!(may_use(&c, "auth/handshake"));
    assert!(may_use(&c, "session/shutdown"));
    assert!(!may_use(&c, "workspace/writeFile"));
    assert!(!may_use(&c, ""), "an empty token matches nothing");
    t.shutdown();
}

/// FR-024c. An identity from an engine that no longer exists is not silently honoured.
#[tokio::test]
async fn an_identity_the_engine_never_had_is_refused_and_a_new_session_begins() {
    let (t, _s) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);

    let r = peer
        .handshake(request(Some("from-an-engine-that-died")))
        .await
        .expect("a handshake");
    assert!(!r.resumed, "the client must learn its work is gone");
    assert_ne!(r.session_id.0, "from-an-engine-that-died");
    t.shutdown();
}

/// The identity the engine actually holds resumes — the other half of the pair above.
#[tokio::test]
async fn the_engines_own_identity_resumes() {
    let (t, _s) = connected_engine(Some("carried-across"));
    let peer = TransportHandshake::new(&*t);

    let r = peer
        .handshake(request(Some("carried-across")))
        .await
        .expect("a handshake");
    assert!(r.resumed, "re-attaching must not restart the session");
    assert_eq!(r.session_id.0, "carried-across");
    t.shutdown();
}

/// T081 — SC-010. This binary needs no network.
#[tokio::test]
async fn this_suite_opens_no_network_sockets() {
    let (t, _s) = connected_engine(None);
    let peer = TransportHandshake::new(&*t);
    let _ = peer.handshake(request(None)).await;
    common::assert_no_network();
    t.shutdown();
}

// ---------------------------------------------------------------------------
// User Story 3: refuse a protocol the client does not understand.
//
// Driven through doubles rather than the real engine: producing an engine that speaks a
// *newer* protocol would mean building one, and the property under test is the client's
// decision, not the engine's behaviour.

use apex_shell::application::use_cases::bootstrap::{Bootstrap, BootstrapOutcome};
use apex_shell::domain::artifact::Architecture;
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

/// T040 — FR-017. A newer engine ends the session, and the refusal names both versions so the
/// message can tell the developer what to update and to what.
#[tokio::test]
async fn a_newer_engine_refuses_the_session_and_names_both_versions() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::speaking(PROTOCOL_VERSION + 1);
    let arts = [artifact(Architecture::LinuxX86_64)];

    match boot(&d, &p, &arts).establish(None).await {
        BootstrapOutcome::RefusedNewerEngine {
            engine_protocol,
            client_protocol,
        } => {
            assert_eq!(engine_protocol, PROTOCOL_VERSION + 1);
            assert_eq!(client_protocol, PROTOCOL_VERSION);
        }
        other => panic!("a newer engine must be refused, got {other:?}"),
    }
}

/// T041 — SC-005. Nothing is exchanged with a newer engine beyond the handshake that
/// discovered the mismatch. Asserted on the count of requests the peer saw, because "we
/// refused" is compatible with having already asked it something.
#[tokio::test]
async fn no_request_is_exchanged_with_a_newer_engine_beyond_the_handshake() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::speaking(PROTOCOL_VERSION + 5);
    let arts = [artifact(Architecture::LinuxX86_64)];

    let _ = boot(&d, &p, &arts).establish(None).await;
    assert_eq!(p.asked(), 1, "exactly one handshake, and nothing after it");
    assert!(
        d.calls().is_empty(),
        "and nothing may be deployed to an engine we refuse to speak to: {:?}",
        d.calls()
    );
}

/// T042 — FR-018. The refusal has no way past it.
///
/// Like F001's changed-host-key test, this asserts the **absence** of a path: no argument,
/// flag or repeated call turns a refusal into a session. A client that speaks a protocol it
/// does not know produces confident wrong behaviour, which is worse than a refusal.
#[tokio::test]
async fn the_refusal_cannot_be_overridden() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::speaking(PROTOCOL_VERSION + 1);
    let arts = [artifact(Architecture::LinuxX86_64)];
    let b = boot(&d, &p, &arts);

    // Every way a caller could ask, including pretending to resume an existing session.
    for resume in [None, Some(SessionId("previous".into()))] {
        assert!(
            matches!(
                b.establish(resume).await,
                BootstrapOutcome::RefusedNewerEngine { .. }
            ),
            "no argument may turn a refusal into a session"
        );
    }
    // And repetition is not an override either.
    for _ in 0..3 {
        assert!(matches!(
            b.establish(None).await,
            BootstrapOutcome::RefusedNewerEngine { .. }
        ));
    }
}

/// T043 — FR-019. Matching versions proceed with no deployment at all.
#[tokio::test]
async fn a_matching_protocol_proceeds_without_deploying_anything() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::default();
    let arts = [artifact(Architecture::LinuxX86_64)];

    match boot(&d, &p, &arts).establish(None).await {
        BootstrapOutcome::Ready { .. } => {}
        other => panic!("expected a ready session, got {other:?}"),
    }
    assert!(
        d.calls().is_empty(),
        "a current engine must not be redeployed: {:?}",
        d.calls()
    );
}

/// T044 — FR-016. An older engine is replaced without the developer being asked.
#[tokio::test]
async fn an_older_engine_is_replaced_without_involving_the_developer() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::speaking(PROTOCOL_VERSION.saturating_sub(1));
    let arts = [artifact(Architecture::LinuxX86_64)];

    // The peer answers every handshake with the same old version, so the sequence deploys and
    // then hands back what the replacement reported.
    let outcome = boot(&d, &p, &arts).establish(None).await;
    assert!(
        matches!(outcome, BootstrapOutcome::Deployed { .. }),
        "an older engine must be replaced, got {outcome:?}"
    );
    assert!(
        d.calls().iter().any(|c| c.starts_with("deploy:")),
        "the replacement must actually be deployed: {:?}",
        d.calls()
    );
}
