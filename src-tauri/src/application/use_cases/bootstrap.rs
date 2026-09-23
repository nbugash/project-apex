//! Getting a working engine onto a host and a compatible session with it (User Story 1).
//!
//! The policy lives here and the mechanism lives behind `ArtifactDeployer`, which is what lets
//! every failure path be exercised without moving a byte. The one rule the whole module
//! enforces: **the client is the authority on protocol version.** It is a comparison, never a
//! negotiation — a common-subset path needs every version to know every other version's
//! capabilities, which is a compatibility matrix nobody maintains correctly.

use crate::application::ports::deployer::{ArtifactDeployer, Target};
use crate::application::ports::handshake::{HandshakeError, HandshakePeer};
use crate::domain::artifact::{Architecture, DeploymentFailure, EngineArtifact};
use apex_protocol::wire::{CapabilitySet, HandshakeRequest, SessionId, PROTOCOL_VERSION};

/// The comparison of two protocol versions, and the whole of the compatibility decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionVerdict {
    Current,
    EngineOlder,
    EngineNewer,
}

impl VersionVerdict {
    pub fn compare(engine: u32, client: u32) -> Self {
        match engine.cmp(&client) {
            std::cmp::Ordering::Equal => Self::Current,
            std::cmp::Ordering::Less => Self::EngineOlder,
            std::cmp::Ordering::Greater => Self::EngineNewer,
        }
    }
}

/// Where the sequence ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapOutcome {
    Ready {
        session_id: SessionId,
        capabilities: CapabilitySet,
        resumed: bool,
    },
    /// The same, after a deployment — distinguished so the interface can say what happened.
    Deployed {
        session_id: SessionId,
        capabilities: CapabilitySet,
    },
    /// FR-017. Carries both numbers so the message can name them.
    RefusedNewerEngine {
        engine_protocol: u32,
        client_protocol: u32,
    },
    Failed(DeploymentFailure),
    HandshakeFailed(String),
}

/// How many times an engine that starts and dies is redeployed before we stop.
///
/// Three, not unlimited: a redeploy loop against a binary that runs and exits is
/// indistinguishable from a hang, and "indefinitely" is not a bound anything can test.
pub const MAX_START_ATTEMPTS: u32 = 3;

/// A zero bound would never deploy at all. Checked at compile time rather than in a test,
/// because a test can only fail after the build has already succeeded.
const _: () = assert!(MAX_START_ATTEMPTS > 0);

/// Generic over the peer rather than holding `&dyn HandshakePeer`: the trait has an `async fn`,
/// which is not dyn-compatible without boxing every future. The deployer stays dynamic because
/// it is synchronous, so the composition root can still swap it freely.
pub struct Bootstrap<'a, P: HandshakePeer> {
    pub deployer: &'a dyn ArtifactDeployer,
    pub peer: &'a P,
    pub artifacts: &'a [EngineArtifact],
    pub target: Target,
    pub client_version: String,
    pub capabilities: CapabilitySet,
}

impl<P: HandshakePeer> Bootstrap<'_, P> {
    /// Choose the artifact this host can run, or refuse by name.
    ///
    /// Refusing rather than falling back to another build: a binary that cannot execute fails
    /// later and less clearly than a refusal that names the architecture.
    pub fn select_artifact(&self, machine: &str) -> Result<&EngineArtifact, DeploymentFailure> {
        let arch = Architecture::from_uname(machine).ok_or_else(|| {
            DeploymentFailure::UnsupportedArchitecture {
                found: machine.trim().to_string(),
            }
        })?;
        self.artifacts
            .iter()
            .find(|a| a.architecture == arch)
            .ok_or(DeploymentFailure::UnsupportedArchitecture {
                found: machine.trim().to_string(),
            })
    }

    fn request(&self, resume: Option<SessionId>) -> HandshakeRequest {
        HandshakeRequest {
            client_version: self.client_version.clone(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: self.capabilities.clone(),
            resume_session: resume,
        }
    }

    /// Deploy, then establish a session.
    ///
    /// Deployment happens first here because this path is entered when the transport has
    /// already classified the engine as missing — F001 produces that classification and, until
    /// this feature, had no recipient for it.
    pub async fn deploy_then_establish(&self) -> BootstrapOutcome {
        let machine = match self.deployer.remote_architecture(&self.target) {
            Ok(m) => m,
            Err(f) => return BootstrapOutcome::Failed(f),
        };
        let artifact = match self.select_artifact(&machine) {
            Ok(a) => a,
            Err(f) => return BootstrapOutcome::Failed(f),
        };
        if let Err(f) = self.deployer.deploy(artifact, &self.target) {
            return BootstrapOutcome::Failed(f);
        }
        match self.peer.handshake(self.request(None)).await {
            Ok(r) => BootstrapOutcome::Deployed {
                session_id: r.session_id,
                capabilities: r.capabilities,
            },
            Err(e) => BootstrapOutcome::HandshakeFailed(e.to_string()),
        }
    }

    /// Establish a session against an engine that is already running.
    pub async fn establish(&self, resume: Option<SessionId>) -> BootstrapOutcome {
        let response = match self.peer.handshake(self.request(resume)).await {
            Ok(r) => r,
            Err(HandshakeError::ConnectionLost) => {
                return BootstrapOutcome::HandshakeFailed(
                    HandshakeError::ConnectionLost.to_string(),
                )
            }
            Err(e) => return BootstrapOutcome::HandshakeFailed(e.to_string()),
        };

        match VersionVerdict::compare(response.protocol_version, PROTOCOL_VERSION) {
            VersionVerdict::Current => BootstrapOutcome::Ready {
                session_id: response.session_id,
                capabilities: response.capabilities,
                resumed: response.resumed,
            },
            // Replaced and re-executed without involving the developer.
            VersionVerdict::EngineOlder => self.deploy_then_establish().await,
            // No override exists. A client that speaks a protocol it does not know produces
            // confident wrong behaviour, which is worse than a refusal naming the problem.
            VersionVerdict::EngineNewer => BootstrapOutcome::RefusedNewerEngine {
                engine_protocol: response.protocol_version,
                client_protocol: PROTOCOL_VERSION,
            },
        }
    }
}

/// Whether a capability may be used. FR-013: a request for one the engine did not advertise
/// fails here, before anything reaches the wire — a request that is sent and rejected produces
/// an error too, so refusing locally is what makes the difference observable.
///
/// A free function rather than an associated one: it does not need a peer, and hanging it off
/// the generic struct would force callers to name a type parameter that has nothing to do with
/// the question being asked.
pub fn may_use(engine: &CapabilitySet, token: &str) -> bool {
    engine.has(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_is_the_authority_on_version() {
        assert_eq!(VersionVerdict::compare(1, 1), VersionVerdict::Current);
        assert_eq!(VersionVerdict::compare(0, 1), VersionVerdict::EngineOlder);
        assert_eq!(VersionVerdict::compare(2, 1), VersionVerdict::EngineNewer);
    }

    /// There is no negotiated middle. Every pair of versions lands in exactly one of three
    /// verdicts, which is what keeps a compatibility matrix from existing.
    #[test]
    fn every_version_pair_reaches_exactly_one_verdict() {
        for engine in 0..6u32 {
            for client in 0..6u32 {
                let v = VersionVerdict::compare(engine, client);
                let expected = match engine.cmp(&client) {
                    std::cmp::Ordering::Equal => VersionVerdict::Current,
                    std::cmp::Ordering::Less => VersionVerdict::EngineOlder,
                    std::cmp::Ordering::Greater => VersionVerdict::EngineNewer,
                };
                assert_eq!(v, expected, "engine {engine} client {client}");
            }
        }
    }

    #[test]
    fn an_unadvertised_capability_is_refused_before_the_wire() {
        let engine = CapabilitySet::of(&["workspace/readFile"]);
        assert!(may_use(&engine, "workspace/readFile"));
        assert!(!may_use(&engine, "workspace/writeFile"));
    }

    #[test]
    fn the_retry_bound_is_a_number_rather_than_indefinitely() {
        assert_eq!(MAX_START_ATTEMPTS, 3);
    }
}
