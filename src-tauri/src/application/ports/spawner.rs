//! Starting a child process.
//!
//! A port because it is the seam that makes failure classification testable. Driving a real
//! `ssh` into a changed host key, a refused credential and a missing engine on demand needs
//! infrastructure the suite is required not to have.

use crate::domain::request::Secret;

/// How to start the child.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub host: String,
    pub user: String,
    /// Phase two of the connect sequence: no `BatchMode`, `SSH_ASKPASS` pointing at the
    /// bundled helper. §3.3 makes the two phases mutually exclusive.
    pub assisted: bool,
}

/// What a spawn produced.
pub struct SpawnedChild {
    pub stdin: Box<dyn std::io::Write + Send>,
    pub stdout: Box<dyn std::io::Read + Send>,
    /// Bounded by `MAX_STDERR_BYTES` — see `domain::failure`.
    pub stderr: Box<dyn std::io::Read + Send>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("the ssh client is not installed")]
    NotFound,
    #[error("ssh {found} is too old; {required} or newer is required")]
    TooOld { found: String, required: String },
    #[error("could not start ssh: {0}")]
    Io(String),
}

pub trait ProcessSpawner: Send + Sync {
    /// Verify the client is present and new enough (FR-005). Checked at startup rather than
    /// discovered at the first connection failure.
    fn preflight(&self) -> Result<String, SpawnError>;

    fn spawn(&self, spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError>;

    /// The invocation this spawner would run. Exposed so a unit test can assert the §3.1
    /// flags are present — the keepalive flags in particular, whose absence no integration
    /// test can catch because the mock has no socket.
    fn invocation(&self, spec: &SpawnSpec) -> Vec<String>;

    /// Hand a passphrase to an assisted attempt in progress.
    fn supply_passphrase(&self, secret: Secret) -> Result<(), SpawnError>;
}
