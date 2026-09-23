//! User Story 1: connect to a machine that has never run the engine.
//!
//! Policy is exercised against doubles. The mechanism — bytes over ssh, a remote `sha256sum`,
//! an atomic rename — is exercised in `bootstrap_real_sshd.rs`, which is opt-in because it
//! needs a real filesystem. Neither proves the other, which is why both exist.

mod common;

use apex_protocol::wire::CapabilitySet;
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
        capabilities: CapabilitySet::of(&["session/shutdown"]),
    }
}

/// T016 — SC-001. A bare host reaches a session with no developer action.
#[tokio::test]
async fn a_host_with_no_engine_reaches_a_session_unaided() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::default();
    let arts = [artifact(Architecture::LinuxX86_64)];

    match boot(&d, &p, &arts).deploy_then_establish().await {
        BootstrapOutcome::Deployed { .. } => {}
        other => panic!("expected a deployed session, got {other:?}"),
    }
    assert!(
        d.calls().iter().any(|c| c.starts_with("deploy:")),
        "an absent engine must be deployed: {:?}",
        d.calls()
    );
}

/// T018 — FR-008. An architecture with no artifact is refused **by name**, before anything
/// transfers. Falling back to another build would defer the failure to the moment the binary
/// will not execute, which is later and far less clear.
#[tokio::test]
async fn an_unsupported_architecture_is_refused_before_any_transfer() {
    let d = ScriptedDeployer::on_architecture("riscv64");
    let p = ScriptedPeer::default();
    let arts = [artifact(Architecture::LinuxX86_64)];

    match boot(&d, &p, &arts).deploy_then_establish().await {
        BootstrapOutcome::Failed(DeploymentFailure::UnsupportedArchitecture { found }) => {
            assert_eq!(found, "riscv64", "the refusal must name what it found");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    assert!(
        !d.calls().iter().any(|c| c.starts_with("deploy:")),
        "nothing may transfer for an architecture we cannot serve: {:?}",
        d.calls()
    );
}

/// The client carries a build for this architecture, but not the one the host runs.
#[tokio::test]
async fn a_known_architecture_with_no_embedded_build_is_still_refused() {
    let d = ScriptedDeployer::on_architecture("aarch64");
    let p = ScriptedPeer::default();
    let arts = [artifact(Architecture::LinuxX86_64)]; // x86 only

    match boot(&d, &p, &arts).deploy_then_establish().await {
        BootstrapOutcome::Failed(DeploymentFailure::UnsupportedArchitecture { found }) => {
            assert_eq!(found, "aarch64");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// T022 — each of the six causes is reported as itself rather than collapsed into a generic
/// failure. Four of them send a developer to four different places.
#[tokio::test]
async fn every_deployment_failure_reaches_the_caller_as_itself() {
    let causes = [
        DeploymentFailure::TransferInterrupted,
        DeploymentFailure::NoSpace,
        DeploymentFailure::DigestMismatch,
        DeploymentFailure::NotExecutable,
        DeploymentFailure::PermissionDenied {
            path: "/home/dev/.apex".into(),
        },
        DeploymentFailure::UnsupportedArchitecture {
            found: "riscv64".into(),
        },
    ];
    for cause in causes {
        let d = ScriptedDeployer::failing(cause.clone());
        let p = ScriptedPeer::default();
        let arts = [artifact(Architecture::LinuxX86_64)];
        match boot(&d, &p, &arts).deploy_then_establish().await {
            BootstrapOutcome::Failed(got) => assert_eq!(got, cause),
            other => panic!("{cause:?} became {other:?}"),
        }
    }
}

/// T017 — SC-004. A failed verification means the artifact is **never executed**.
///
/// Asserted on the handshake count, not on the error returned: a deployment that ran the
/// binary and then reported an error also returns an error, so the error alone proves nothing.
/// A handshake attempt is the observable proof that something was started.
#[tokio::test]
async fn an_artifact_that_fails_verification_is_never_executed() {
    let d = ScriptedDeployer::failing(DeploymentFailure::DigestMismatch);
    let p = ScriptedPeer::default();
    let arts = [artifact(Architecture::LinuxX86_64)];

    let outcome = boot(&d, &p, &arts).deploy_then_establish().await;
    assert_eq!(
        outcome,
        BootstrapOutcome::Failed(DeploymentFailure::DigestMismatch)
    );
    assert_eq!(
        p.asked(),
        0,
        "a failed verification must not be followed by starting the engine"
    );
    assert!(
        !d.calls().iter().any(|c| c.starts_with("retire:")),
        "and nothing may be retired on a failed deployment"
    );
}

/// A handshake that never completes is reported as itself, not as a deployment failure. The
/// bytes arrived; the engine is wedged. Different problem, different remedy.
#[tokio::test]
async fn an_engine_that_will_not_answer_is_not_reported_as_a_failed_deployment() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::failing(HandshakeError::TimedOut);
    let arts = [artifact(Architecture::LinuxX86_64)];

    match boot(&d, &p, &arts).deploy_then_establish().await {
        BootstrapOutcome::HandshakeFailed(m) => assert!(m.contains("did not answer"), "{m}"),
        other => panic!("expected a handshake failure, got {other:?}"),
    }
}

/// T079 — SC-010. This binary needs no network.
#[tokio::test]
async fn this_suite_opens_no_network_sockets() {
    let d = ScriptedDeployer::default();
    let p = ScriptedPeer::default();
    let arts = [artifact(Architecture::LinuxX86_64)];
    let _ = boot(&d, &p, &arts).deploy_then_establish().await;
    common::assert_no_network();
}
