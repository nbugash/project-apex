//! Shared test doubles.
//!
//! Placed here rather than in `tests/mock_daemon/` as tasks.md first said: that target is
//! `harness = false`, so it is a binary the transport spawns, and nothing can import from
//! it. Integration tests are separate crates, and `tests/common/` is how Rust shares code
//! between them.

use apex_shell::application::ports::spawner::{
    ProcessSpawner, SpawnError, SpawnSpec, SpawnedChild,
};
use apex_shell::domain::request::Secret;
use std::sync::{Arc, Mutex};

/// Produces a chosen exit code and chosen stderr without running `ssh`.
///
/// This is the seam that makes failure classification testable. Provoking a changed host
/// key, a refused credential and a missing engine from a real `ssh` on demand would need a
/// remote host that will do each of those things — three hosts, or one repeatedly
/// reconfigured, over a network the suite is required not to need.
pub struct ScriptedSpawner {
    pub exit_code: i32,
    pub stderr: String,
    pub version: Result<String, SpawnError>,
    /// Every passphrase handed to it, so a test can assert one was or was not requested.
    pub passphrases: Arc<Mutex<Vec<usize>>>,
    /// Records each invocation, so a test can assert the §3.1 flags are present.
    pub invocations: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Default for ScriptedSpawner {
    fn default() -> Self {
        Self {
            exit_code: 0,
            stderr: String::new(),
            version: Ok("OpenSSH_9.6p1".into()),
            passphrases: Arc::new(Mutex::new(Vec::new())),
            invocations: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ScriptedSpawner {
    pub fn failing(exit_code: i32, stderr: &str) -> Self {
        Self {
            exit_code,
            stderr: stderr.into(),
            ..Default::default()
        }
    }

    pub fn with_version(version: Result<String, SpawnError>) -> Self {
        Self {
            version,
            ..Default::default()
        }
    }

    pub fn passphrase_count(&self) -> usize {
        self.passphrases.lock().expect("passphrase lock").len()
    }
}

impl ProcessSpawner for ScriptedSpawner {
    fn preflight(&self) -> Result<String, SpawnError> {
        match &self.version {
            Ok(v) => Ok(v.clone()),
            Err(SpawnError::NotFound) => Err(SpawnError::NotFound),
            Err(SpawnError::TooOld { found, required }) => Err(SpawnError::TooOld {
                found: found.clone(),
                required: required.clone(),
            }),
            Err(SpawnError::Io(m)) => Err(SpawnError::Io(m.clone())),
        }
    }

    fn spawn(&self, spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        self.invocations
            .lock()
            .expect("invocation lock")
            .push(self.invocation(spec));
        // A scripted spawner never produces a working child: its whole purpose is the
        // failure path. A test that needs a working pipe uses the mock daemon instead.
        Err(SpawnError::Io(format!(
            "scripted failure: exit {} — {}",
            self.exit_code, self.stderr
        )))
    }

    fn invocation(&self, spec: &SpawnSpec) -> Vec<String> {
        // Mirrors the real spawner's shape closely enough for flag assertions.
        let mut v: Vec<String> = [
            "ssh",
            "-o",
            "ControlMaster=auto",
            "-o",
            "ControlPath=~/.ssh/apex-%C",
            "-o",
            "ControlPersist=1h",
            "-o",
            "IPQoS=throughput",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "StrictHostKeyChecking=accept-new",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if !spec.assisted {
            v.push("-o".into());
            v.push("BatchMode=yes".into());
        }
        v.push(format!("{}@{}", spec.user, spec.host));
        v
    }

    fn supply_passphrase(&self, secret: Secret) -> Result<(), SpawnError> {
        self.passphrases
            .lock()
            .expect("passphrase lock")
            .push(secret.len());
        Ok(())
    }
}

/// Scripted stderr fixtures, as OpenSSH emits them under `LC_ALL=C`.
pub mod stderr {
    pub const TIMED_OUT: &str = "ssh: connect to host example.com port 22: Connection timed out";
    pub const REFUSED: &str = "ssh: connect to host example.com port 22: Connection refused";
    pub const DENIED: &str = "user@example.com: Permission denied (publickey,password).";
    pub const HOST_KEY_CHANGED: &str = concat!(
        "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n",
        "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n",
        "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@"
    );
    pub const NOT_FOUND: &str = "bash: /usr/local/bin/ide-engine: No such file or directory";
    /// The same refusal a French system emits. Classification must be identical (SC-008),
    /// which is what `LC_ALL=C` in the invocation is for — if it were ever dropped, this is
    /// what the transport would be asked to parse.
    pub const DENIED_FR: &str = "user@example.com: Permission refusée (publickey,password).";
}

// ---------------------------------------------------------------------------

use apex_shell::adapters::outbound::openssh::SshTransport;
use std::process::{Child, Command, Stdio};

/// Spawns the mock daemon instead of `ssh`.
///
/// The transport under test is the real one: a real child process, a real pipe, a real
/// framing codec. Only what is on the other end differs, which is the point of the
/// `ProcessSpawner` port.
pub struct MockSpawner {
    pub script: String,
    children: Arc<Mutex<Vec<Child>>>,
}

impl MockSpawner {
    pub fn new(script: &str) -> Self {
        Self {
            script: script.into(),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// How many mock processes are still running. SC-003 asserts this reaches zero.
    pub fn live_children(&self) -> usize {
        let mut guard = self.children.lock().expect("children lock");
        guard.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        guard.len()
    }
}

impl ProcessSpawner for MockSpawner {
    fn preflight(&self) -> Result<String, SpawnError> {
        Ok("OpenSSH_9.6p1 (mock)".into())
    }

    fn invocation(&self, spec: &SpawnSpec) -> Vec<String> {
        vec![format!("{}@{}", spec.user, spec.host)]
    }

    fn spawn(&self, _spec: &SpawnSpec) -> Result<SpawnedChild, SpawnError> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_apex-mock-daemon"))
            .env("APEX_MOCK_SCRIPT", &self.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SpawnError::Io(e.to_string()))?;

        let out = SpawnedChild {
            stdin: Box::new(child.stdin.take().expect("piped stdin")),
            stdout: Box::new(child.stdout.take().expect("piped stdout")),
            stderr: Box::new(child.stderr.take().expect("piped stderr")),
        };
        self.children.lock().expect("children lock").push(child);
        Ok(out)
    }

    fn supply_passphrase(&self, _secret: Secret) -> Result<(), SpawnError> {
        Ok(())
    }
}

pub fn spec() -> SpawnSpec {
    SpawnSpec {
        host: "mock.invalid".into(),
        user: "dev".into(),
        assisted: false,
    }
}

/// A connected transport speaking to a mock driven by `script`.
pub fn connected(script: &str) -> (Arc<SshTransport>, Arc<MockSpawner>) {
    let spawner = Arc::new(MockSpawner::new(script));
    let t = Arc::new(SshTransport::new(spawner.clone(), spec()));
    t.connect().expect("the mock should connect");
    (t, spawner)
}
