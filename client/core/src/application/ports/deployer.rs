//! Outbound port: put an engine on a host.
//!
//! A port because the mechanism is the part that needs a real host, and the policy is the part
//! that needs testing. Guarantees are in specs/004-daemon-bootstrap/contracts/deployment.md.

use crate::domain::artifact::{DeploymentFailure, DeploymentState, EngineArtifact};

/// Where an engine is being deployed.
#[derive(Debug, Clone)]
pub struct Target {
    pub host: String,
    pub user: String,
}

pub trait ArtifactDeployer: Send + Sync {
    /// Discover what the remote host runs, so the right artifact can be chosen.
    fn remote_architecture(&self, target: &Target) -> Result<String, DeploymentFailure>;

    /// Stage, verify and promote. On success the artifact is present and executable; any
    /// previous engine is untouched.
    fn deploy(&self, artifact: &EngineArtifact, target: &Target) -> Result<(), DeploymentFailure>;

    /// Remove an engine a replacement superseded.
    ///
    /// Separate from `deploy` because it may only run after a handshake has proven the
    /// replacement works — and a deployer cannot observe a handshake. A deployer that retired
    /// the old engine on promotion would break the guarantee that a verified-but-unrunnable
    /// binary is survivable.
    fn retire_previous(&self, version: &str, target: &Target) -> Result<(), DeploymentFailure>;

    /// Progress, published at least once per second while transferring.
    fn observe(&self) -> tokio::sync::watch::Receiver<DeploymentState>;
}
