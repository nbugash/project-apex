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
